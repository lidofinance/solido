// SPDX-FileCopyrightText: 2021 Chorus One AG
// SPDX-License-Identifier: GPL-3.0

use std::{fmt, path::PathBuf};

use serde::Serialize;
use solana_program::{pubkey::Pubkey, system_instruction};
use solana_sdk::{
    account::ReadableAccount,
    signature::{Keypair, Signer},
};

use lido::{
    balance::get_validator_to_withdraw,
    find_authority_program_address,
    metrics::LamportsHistogram,
    processor::StakeType,
    state::{
        AccountList, Criteria, Lido, ListEntry, Maintainer, RewardDistribution, SeedRange,
        Validator, ValidatorPerf,
    },
    token::{Lamports, StLamports},
    util::serialize_b58,
    vote_state::get_vote_account_commission,
    MINT_AUTHORITY, RESERVE_ACCOUNT, STAKE_AUTHORITY,
};
use solido_cli_common::{
    error::{CliError, Error},
    per64::to_f64,
    snapshot::{SnapshotClientConfig, SnapshotConfig},
    validator_info_utils::ValidatorInfo,
};

use crate::{
    commands_multisig::{
        get_multisig_program_address, propose_instruction, ProposeInstructionOutput,
    },
    config::RemoveValidatorOpts,
    spl_token_utils::{push_create_spl_token_account, push_create_spl_token_mint},
};
use crate::{
    config::{
        AddRemoveMaintainerOpts, AddValidatorOpts, ChangeCriteriaOpts, CreateSolidoOpts,
        CreateV2AccountsOpts, DeactivateIfViolatesOpts, DeactivateValidatorOpts, DepositOpts,
        MigrateStateToV2Opts, ShowSolidoAuthoritiesOpts, ShowSolidoOpts, WithdrawOpts,
    },
    get_signer_from_path,
};

#[derive(Serialize)]
pub struct CreateSolidoOutput {
    /// Account that stores the data for this Solido instance.
    #[serde(serialize_with = "serialize_b58")]
    pub solido_address: Pubkey,

    /// Manages the deposited sol.
    #[serde(serialize_with = "serialize_b58")]
    pub reserve_account: Pubkey,

    /// SPL token mint account for StSol tokens.
    #[serde(serialize_with = "serialize_b58")]
    pub st_sol_mint_address: Pubkey,

    /// stSOL SPL token account that holds the treasury funds.
    #[serde(serialize_with = "serialize_b58")]
    pub treasury_account: Pubkey,

    /// stSOL SPL token account that receives the developer fees.
    #[serde(serialize_with = "serialize_b58")]
    pub developer_account: Pubkey,

    /// Authority for the minting.
    #[serde(serialize_with = "serialize_b58")]
    pub mint_authority: Pubkey,

    /// Data account that holds list of validators
    #[serde(serialize_with = "serialize_b58")]
    pub validator_list_address: Pubkey,

    /// Data account that holds list of validators
    #[serde(serialize_with = "serialize_b58")]
    pub validator_perf_list_address: Pubkey,

    /// Data account that holds list of maintainers
    #[serde(serialize_with = "serialize_b58")]
    pub maintainer_list_address: Pubkey,
}

impl fmt::Display for CreateSolidoOutput {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "Solido details:")?;
        writeln!(
            f,
            "  Solido address:                {}",
            self.solido_address
        )?;
        writeln!(
            f,
            "  Reserve account:               {}",
            self.reserve_account
        )?;
        writeln!(
            f,
            "  Mint authority:                {}",
            self.mint_authority
        )?;
        writeln!(
            f,
            "  stSOL mint:                    {}",
            self.st_sol_mint_address
        )?;
        writeln!(
            f,
            "  Treasury SPL token account:    {}",
            self.treasury_account
        )?;
        writeln!(
            f,
            "  Developer fee SPL token account: {}",
            self.developer_account
        )?;
        Ok(())
    }
}

/// Get keypair from key path or random if not set
fn from_key_path_or_random(key_path: &PathBuf) -> solido_cli_common::Result<Box<dyn Signer>> {
    let lido_signer = {
        if key_path != &PathBuf::default() {
            // If we've been given a solido private key, use it to create the solido instance.
            get_signer_from_path(key_path.clone())?
        } else {
            // If not, use a random key
            Box::new(Keypair::new())
        }
    };
    Ok(lido_signer)
}

pub fn command_create_solido(
    config: &mut SnapshotConfig,
    opts: &CreateSolidoOpts,
) -> solido_cli_common::Result<CreateSolidoOutput> {
    let lido_signer = from_key_path_or_random(opts.solido_key_path())?;
    let validator_list_signer = from_key_path_or_random(opts.validator_list_key_path())?;
    let validator_perf_list_signer = from_key_path_or_random(opts.validator_perf_list_key_path())?;
    let maintainer_list_signer = from_key_path_or_random(opts.maintainer_list_key_path())?;

    let (reserve_account, _) = lido::find_authority_program_address(
        opts.solido_program_id(),
        &lido_signer.pubkey(),
        RESERVE_ACCOUNT,
    );

    let (mint_authority, _) = lido::find_authority_program_address(
        opts.solido_program_id(),
        &lido_signer.pubkey(),
        MINT_AUTHORITY,
    );

    let (manager, _nonce) =
        get_multisig_program_address(opts.multisig_program_id(), opts.multisig_address());

    let lido_size = Lido::calculate_size();
    let lido_account_balance = config
        .client
        .get_minimum_balance_for_rent_exemption(lido_size)?;

    let validator_list_size = AccountList::<Validator>::required_bytes(*opts.max_validators());
    let validator_list_account_balance = config
        .client
        .get_minimum_balance_for_rent_exemption(validator_list_size)?;

    let validator_perf_list_size =
        AccountList::<ValidatorPerf>::required_bytes(*opts.max_validators());
    let validator_perf_list_account_balance = config
        .client
        .get_minimum_balance_for_rent_exemption(validator_perf_list_size)?;

    let maintainer_list_size = AccountList::<Maintainer>::required_bytes(*opts.max_maintainers());
    let maintainer_list_account_balance = config
        .client
        .get_minimum_balance_for_rent_exemption(maintainer_list_size)?;

    let mut instructions = Vec::new();

    // We need to fund Lido's reserve account so it is rent-exempt, otherwise it
    // might disappear.
    let min_balance_empty_data_account = config.client.get_minimum_balance_for_rent_exemption(0)?;
    instructions.push(system_instruction::transfer(
        &config.signer.pubkey(),
        &reserve_account,
        min_balance_empty_data_account.0,
    ));

    let st_sol_mint_pubkey = {
        if opts.mint_address() != &Pubkey::default() {
            // If we've been given a minter address, return its public key.
            *opts.mint_address()
        } else {
            // If not, set up the Lido stSOL SPL token mint account.
            let st_sol_mint_keypair =
                push_create_spl_token_mint(config, &mut instructions, &mint_authority)?;
            let signers = &[&st_sol_mint_keypair, config.signer];
            // Ideally we would set up the entire instance in a single transaction, but
            // Solana transaction size limits are so low that we need to break our
            // instructions down into multiple transactions. So set up the mint first,
            // then continue.
            config.sign_and_send_transaction(&instructions[..], signers)?;
            instructions.clear();
            eprintln!("Did send mint init.");
            st_sol_mint_keypair.pubkey()
        }
    };

    // Set up the SPL token account that receive the fees in stSOL.
    let treasury_keypair = push_create_spl_token_account(
        config,
        &mut instructions,
        &st_sol_mint_pubkey,
        opts.treasury_account_owner(),
    )?;
    let developer_keypair = push_create_spl_token_account(
        config,
        &mut instructions,
        &st_sol_mint_pubkey,
        opts.developer_account_owner(),
    )?;
    config.sign_and_send_transaction(
        &instructions[..],
        &vec![config.signer, &treasury_keypair, &developer_keypair],
    )?;
    instructions.clear();
    eprintln!("Did send SPL account inits.");

    // Create the account that holds the Solido instance itself.
    instructions.push(system_instruction::create_account(
        &config.signer.pubkey(),
        &lido_signer.pubkey(),
        lido_account_balance.0,
        lido_size as u64,
        opts.solido_program_id(),
    ));

    // Create the account that holds the validator list itself.
    instructions.push(system_instruction::create_account(
        &config.signer.pubkey(),
        &validator_list_signer.pubkey(),
        validator_list_account_balance.0,
        validator_list_size as u64,
        opts.solido_program_id(),
    ));

    // Create the account that holds the validator perf list itself.
    instructions.push(system_instruction::create_account(
        &config.signer.pubkey(),
        &validator_perf_list_signer.pubkey(),
        validator_perf_list_account_balance.0,
        validator_perf_list_size as u64,
        opts.solido_program_id(),
    ));

    // Create the account that holds the maintainer list itself.
    instructions.push(system_instruction::create_account(
        &config.signer.pubkey(),
        &maintainer_list_signer.pubkey(),
        maintainer_list_account_balance.0,
        maintainer_list_size as u64,
        opts.solido_program_id(),
    ));

    instructions.push(lido::instruction::initialize(
        opts.solido_program_id(),
        RewardDistribution {
            treasury_fee: *opts.treasury_fee_share(),
            developer_fee: *opts.developer_fee_share(),
            st_sol_appreciation: *opts.st_sol_appreciation_share(),
        },
        Criteria {
            max_commission: *opts.max_commission(),
            min_block_production_rate: *opts.min_block_production_rate(),
            min_vote_success_rate: *opts.min_vote_success_rate(),
        },
        *opts.max_validators(),
        *opts.max_maintainers(),
        &lido::instruction::InitializeAccountsMeta {
            lido: lido_signer.pubkey(),
            manager,
            st_sol_mint: st_sol_mint_pubkey,
            treasury_account: treasury_keypair.pubkey(),
            developer_account: developer_keypair.pubkey(),
            reserve_account,
            validator_list: validator_list_signer.pubkey(),
            validator_perf_list: validator_perf_list_signer.pubkey(),
            maintainer_list: maintainer_list_signer.pubkey(),
        },
    ));

    config.sign_and_send_transaction(
        &instructions[..],
        &vec![
            config.signer,
            &*lido_signer,
            &*validator_list_signer,
            &*validator_perf_list_signer,
            &*maintainer_list_signer,
        ],
    )?;
    eprintln!("Did send Lido init.");

    let result = CreateSolidoOutput {
        solido_address: lido_signer.pubkey(),
        reserve_account,
        mint_authority,
        st_sol_mint_address: st_sol_mint_pubkey,
        treasury_account: treasury_keypair.pubkey(),
        developer_account: developer_keypair.pubkey(),
        validator_list_address: validator_list_signer.pubkey(),
        validator_perf_list_address: validator_perf_list_signer.pubkey(),
        maintainer_list_address: maintainer_list_signer.pubkey(),
    };
    Ok(result)
}

/// CLI entry point to add a validator to Solido.
pub fn command_add_validator(
    config: &mut SnapshotConfig,
    opts: &AddValidatorOpts,
) -> solido_cli_common::Result<ProposeInstructionOutput> {
    let (multisig_address, _) =
        get_multisig_program_address(opts.multisig_program_id(), opts.multisig_address());

    let solido = config.client.get_solido(opts.solido_address())?;

    let instruction = lido::instruction::add_validator(
        opts.solido_program_id(),
        &lido::instruction::AddValidatorMetaV2 {
            lido: *opts.solido_address(),
            manager: multisig_address,
            validator_vote_account: *opts.validator_vote_account(),
            validator_list: solido.validator_list,
        },
    );
    propose_instruction(
        config,
        opts.multisig_program_id(),
        *opts.multisig_address(),
        instruction,
    )
}

/// CLI entry point to deactivate a validator.
pub fn command_deactivate_validator(
    config: &mut SnapshotConfig,
    opts: &DeactivateValidatorOpts,
) -> solido_cli_common::Result<ProposeInstructionOutput> {
    let (multisig_address, _) =
        get_multisig_program_address(opts.multisig_program_id(), opts.multisig_address());

    let solido = config.client.get_solido(opts.solido_address())?;
    let validators = config
        .client
        .get_account_list::<Validator>(&solido.validator_list)?;

    let validator_index = validators
        .position(opts.validator_vote_account())
        .ok_or_else(|| CliError::new("Pubkey not found in validator list"))?;

    let instruction = lido::instruction::deactivate_validator(
        opts.solido_program_id(),
        &lido::instruction::DeactivateValidatorMetaV2 {
            lido: *opts.solido_address(),
            manager: multisig_address,
            validator_vote_account_to_deactivate: *opts.validator_vote_account(),
            validator_list: solido.validator_list,
        },
        validator_index,
    );
    propose_instruction(
        config,
        opts.multisig_program_id(),
        *opts.multisig_address(),
        instruction,
    )
}

/// CLI entry point to to add a maintainer to Solido.
pub fn command_add_maintainer(
    config: &mut SnapshotConfig,
    opts: &AddRemoveMaintainerOpts,
) -> solido_cli_common::Result<ProposeInstructionOutput> {
    let (multisig_address, _) =
        get_multisig_program_address(opts.multisig_program_id(), opts.multisig_address());

    let solido = config.client.get_solido(opts.solido_address())?;

    let instruction = lido::instruction::add_maintainer(
        opts.solido_program_id(),
        &lido::instruction::AddMaintainerMetaV2 {
            lido: *opts.solido_address(),
            manager: multisig_address,
            maintainer: *opts.maintainer_address(),
            maintainer_list: solido.maintainer_list,
        },
    );
    propose_instruction(
        config,
        opts.multisig_program_id(),
        *opts.multisig_address(),
        instruction,
    )
}

/// Command to add a validator to Solido.
pub fn command_remove_maintainer(
    config: &mut SnapshotConfig,
    opts: &AddRemoveMaintainerOpts,
) -> solido_cli_common::Result<ProposeInstructionOutput> {
    let (multisig_address, _) =
        get_multisig_program_address(opts.multisig_program_id(), opts.multisig_address());

    let solido = config.client.get_solido(opts.solido_address())?;
    let maintainers = config
        .client
        .get_account_list::<Maintainer>(&solido.maintainer_list)?;

    let maintainer_index = maintainers
        .position(opts.maintainer_address())
        .ok_or_else(|| CliError::new("Pubkey not found in maintainer list"))?;

    let instruction = lido::instruction::remove_maintainer(
        opts.solido_program_id(),
        &lido::instruction::RemoveMaintainerMetaV2 {
            lido: *opts.solido_address(),
            manager: multisig_address,
            maintainer: *opts.maintainer_address(),
            maintainer_list: solido.maintainer_list,
        },
        maintainer_index,
    );
    propose_instruction(
        config,
        opts.multisig_program_id(),
        *opts.multisig_address(),
        instruction,
    )
}

/// `Validator` structure with all the fields from its related structs
/// joined by its `Pubkey`.
#[derive(Serialize)]
pub struct RichValidator {
    #[serde(serialize_with = "serialize_b58")]
    pub vote_account_address: Pubkey,
    pub stake_seeds: SeedRange,
    pub unstake_seeds: SeedRange,
    pub stake_accounts_balance: Lamports,
    pub unstake_accounts_balance: Lamports,
    pub effective_stake_balance: Lamports,
    pub active: bool,

    #[serde(serialize_with = "serialize_b58")]
    pub identity_account_address: Pubkey,

    pub info: ValidatorInfo,
    pub perf: Option<ValidatorPerf>,

    pub commission: u8,
}

#[derive(Serialize)]
pub struct ShowSolidoOutput {
    pub solido: Lido,

    #[serde(serialize_with = "serialize_b58")]
    pub solido_program_id: Pubkey,

    #[serde(serialize_with = "serialize_b58")]
    pub solido_address: Pubkey,

    #[serde(serialize_with = "serialize_b58")]
    pub reserve_account: Pubkey,

    #[serde(serialize_with = "serialize_b58")]
    pub stake_authority: Pubkey,

    #[serde(serialize_with = "serialize_b58")]
    pub mint_authority: Pubkey,

    /// Validator structure as the program sees it, along with the validator's
    /// identity account address, their info, their performance data,
    /// and their commission percentage.
    pub validators: Vec<RichValidator>,
    pub validators_max: u32,

    pub maintainers: Vec<Maintainer>,
    pub maintainers_max: u32,

    pub reserve_account_balance: Lamports,
}

pub const VALIDATOR_STAKE_ACCOUNT: &[u8] = b"validator_stake_account";
pub const VALIDATOR_UNSTAKE_ACCOUNT: &[u8] = b"validator_unstake_account";

fn find_stake_account_address_with_authority(
    vote_account_address: &Pubkey,
    program_id: &Pubkey,
    solido_account: &Pubkey,
    authority: &[u8],
    seed: u64,
) -> (Pubkey, u8) {
    let seeds = [
        &solido_account.to_bytes(),
        &vote_account_address.to_bytes(),
        authority,
        &seed.to_le_bytes()[..],
    ];
    Pubkey::find_program_address(&seeds, program_id)
}

fn find_stake_account_address(
    vote_account_address: &Pubkey,
    program_id: &Pubkey,
    solido_account: &Pubkey,
    seed: u64,
    stake_type: StakeType,
) -> (Pubkey, u8) {
    let authority = match stake_type {
        StakeType::Stake => VALIDATOR_STAKE_ACCOUNT,
        StakeType::Unstake => VALIDATOR_UNSTAKE_ACCOUNT,
    };
    find_stake_account_address_with_authority(
        vote_account_address,
        program_id,
        solido_account,
        authority,
        seed,
    )
}

impl fmt::Display for ShowSolidoOutput {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "Manager:                     {}", self.solido.manager)?;
        writeln!(
            f,
            "stSOL mint:                  {}",
            self.solido.st_sol_mint
        )?;

        writeln!(f, "\nExchange rate:")?;
        writeln!(
            f,
            "  Computed in epoch: {}",
            self.solido.exchange_rate.computed_in_epoch
        )?;
        writeln!(
            f,
            "  SOL balance:       {}",
            self.solido.exchange_rate.sol_balance
        )?;
        writeln!(
            f,
            "  stSOL supply:      {}",
            self.solido.exchange_rate.st_sol_supply
        )?;

        writeln!(f, "\nReserve balance: {}", self.reserve_account_balance)?;

        writeln!(f, "\nAuthorities (public key, bump seed):")?;
        writeln!(
            f,
            "Stake authority:            {}, {}",
            self.stake_authority, self.solido.stake_authority_bump_seed
        )?;
        writeln!(
            f,
            "Mint authority:             {}, {}",
            self.mint_authority, self.solido.mint_authority_bump_seed
        )?;
        writeln!(
            f,
            "Reserve:                    {}, {}",
            self.reserve_account, self.solido.sol_reserve_account_bump_seed
        )?;
        writeln!(f, "\nReward distribution:")?;
        let mut print_reward = |name, get: fn(&RewardDistribution) -> u32| {
            writeln!(
                f,
                "  {:4}/{:4} => {}",
                get(&self.solido.reward_distribution),
                self.solido.reward_distribution.sum(),
                name,
            )
        };
        print_reward("stSOL appreciation", |d| d.st_sol_appreciation)?;
        print_reward("Treasury", |d| d.treasury_fee)?;
        print_reward("Developer fee", |d| d.developer_fee)?;

        writeln!(f, "\nFee recipients:")?;
        writeln!(
            f,
            "  Treasury SPL token account:      {}",
            self.solido.fee_recipients.treasury_account
        )?;
        writeln!(
            f,
            "  Developer fee SPL token account: {}",
            self.solido.fee_recipients.developer_account
        )?;

        writeln!(f, "\nMetrics:")?;
        writeln!(
            f,
            "  Total treasury fee:       {}, valued at {} when it was paid",
            self.solido.metrics.fee_treasury_st_sol_total,
            self.solido.metrics.fee_treasury_sol_total,
        )?;
        writeln!(
            f,
            "  Total developer fee:      {}, valued at {} when it was paid",
            self.solido.metrics.fee_developer_st_sol_total,
            self.solido.metrics.fee_developer_sol_total,
        )?;
        writeln!(
            f,
            "  Total stSOL appreciation: {}",
            self.solido.metrics.st_sol_appreciation_sol_total
        )?;
        writeln!(
            f,
            "  Total amount withdrawn:   {}, valued at {} when it was withdrawn",
            self.solido.metrics.withdraw_amount.total_st_sol_amount,
            self.solido.metrics.withdraw_amount.total_sol_amount,
        )?;
        writeln!(
            f,
            "  Number of withdrawals:    {}",
            self.solido.metrics.withdraw_amount.count,
        )?;
        writeln!(
            f,
            "  Total deposited:          {}",
            self.solido.metrics.deposit_amount.total
        )?;
        for (count, upper_bound) in self
            .solido
            .metrics
            .deposit_amount
            .counts
            .iter()
            .zip(&LamportsHistogram::BUCKET_UPPER_BOUNDS)
        {
            writeln!(
                f,
                "  Number of deposits ≤ {:>25}: {}",
                format!("{}", upper_bound),
                count
            )?;
        }

        writeln!(f, "\nValidator curation criteria:")?;
        writeln!(
            f,
            "  Max validation commission: {}%",
            self.solido.criteria.max_commission,
        )?;
        writeln!(
            f,
            "  Min block production rate: {:.2}%",
            100.0 * to_f64(self.solido.criteria.min_block_production_rate),
        )?;
        writeln!(
            f,
            "  Min vote success rate:     {:.2}%",
            100.0 * to_f64(self.solido.criteria.min_vote_success_rate),
        )?;

        writeln!(f, "\nValidator list {}", self.solido.validator_list)?;
        writeln!(
            f,
            "Validators: {} in use out of {} that the instance can support",
            self.validators.len(),
            self.validators_max,
        )?;
        for v in self.validators.iter() {
            writeln!(
                f,
                "\n  - \
                Name:                      {}\n    \
                Keybase username:          {}\n    \
                Vote account:              {}\n    \
                Identity account:          {}\n    \
                Commission:                {}%\n    \
                Active:                    {}\n    \
                Stake in all accounts:     {}\n    \
                Stake in stake accounts:   {}\n    \
                Stake in unstake accounts: {}",
                v.info.name,
                match &v.info.keybase_username {
                    Some(username) => &username[..],
                    None => "not set",
                },
                v.vote_account_address,
                v.identity_account_address,
                v.commission,
                v.active,
                v.stake_accounts_balance,
                v.effective_stake_balance,
                v.unstake_accounts_balance,
            )?;

            writeln!(f, "    Stake accounts (seed, address):")?;
            if v.stake_seeds.begin == v.stake_seeds.end {
                writeln!(f, "      This validator has no stake accounts.")?;
            };
            for seed in &v.stake_seeds {
                writeln!(
                    f,
                    "      - {}: {}",
                    seed,
                    find_stake_account_address(
                        &v.vote_account_address,
                        &self.solido_program_id,
                        &self.solido_address,
                        seed,
                        StakeType::Stake,
                    )
                    .0
                )?;
            }

            writeln!(f, "    Unstake accounts (seed, address):")?;
            if v.unstake_seeds.begin == v.unstake_seeds.end {
                writeln!(f, "      This validator has no unstake accounts.")?;
            };
            for seed in &v.unstake_seeds {
                writeln!(
                    f,
                    "      - {}: {}",
                    seed,
                    find_stake_account_address(
                        &v.vote_account_address,
                        &self.solido_program_id,
                        &self.solido_address,
                        seed,
                        StakeType::Unstake,
                    )
                    .0
                )?;
            }

            writeln!(f, "    Off-chain performance readings:")?;
            if let Some(Some(perf)) = v.perf.as_ref().map(|perf| &perf.rest) {
                writeln!(
                    f,
                    "      For epoch                  #{}", // --
                    perf.updated_at,
                )?;
                writeln!(
                    f,
                    "      Block Production Rate:      {:.2}%",
                    100.0 * to_f64(perf.block_production_rate)
                )?;
                writeln!(
                    f,
                    "      Vote Success Rate:          {:.2}%",
                    100.0 * to_f64(perf.vote_success_rate)
                )?;
            } else {
                writeln!(f, "      Not yet collected.")?;
            }
            writeln!(f, "    On-chain performance readings:")?;
            if let Some(perf) = &v.perf {
                writeln!(
                    f,
                    "      For epoch                  #{}",
                    perf.commission_updated_at,
                )?;
                writeln!(
                    f,
                    "      Worst Commission:           {}%", // --
                    perf.commission
                )?;
            } else {
                writeln!(f, "      Not yet collected.")?;
            }
        }
        writeln!(f, "\nMaintainer list {}", self.solido.maintainer_list)?;
        writeln!(
            f,
            "Maintainers: {} in use out of {} that the instance can support\n",
            self.maintainers.len(),
            self.maintainers_max,
        )?;
        for e in &self.maintainers {
            writeln!(f, "  - {}", e.pubkey())?;
        }
        Ok(())
    }
}

pub fn command_show_solido(
    config: &mut SnapshotConfig,
    opts: &ShowSolidoOpts,
) -> solido_cli_common::Result<ShowSolidoOutput> {
    let lido = config.client.get_solido(opts.solido_address())?;
    let reserve_account =
        lido.get_reserve_account(opts.solido_program_id(), opts.solido_address())?;
    let stake_authority =
        lido.get_stake_authority(opts.solido_program_id(), opts.solido_address())?;
    let mint_authority =
        lido.get_mint_authority(opts.solido_program_id(), opts.solido_address())?;

    let reserve_account_balance = config.client.get_account(&reserve_account)?.lamports;

    let validators = config
        .client
        .get_account_list::<Validator>(&lido.validator_list)?;
    let available_perfs = config
        .client
        .get_account_list::<ValidatorPerf>(&lido.validator_perf_list)?;
    let validators_max = validators.header.max_entries;
    let validators = validators.entries;
    let maintainers = config
        .client
        .get_account_list::<Maintainer>(&lido.maintainer_list)?;
    let maintainers_max = maintainers.header.max_entries;
    let maintainers = maintainers.entries;

    let mut validator_identities = Vec::new();
    let mut validator_infos = Vec::new();
    let mut validator_commission_percentages = Vec::new();
    let mut validator_perfs = Vec::new();
    for validator in validators.iter() {
        let vote_state = config.client.get_vote_account(validator.pubkey())?;
        validator_identities.push(vote_state.node_pubkey);
        let info = config.client.get_validator_info(&vote_state.node_pubkey)?;
        validator_infos.push(info);
        let vote_account = config.client.get_account(validator.pubkey())?;
        let commission = get_vote_account_commission(&vote_account.data)
            .ok()
            .ok_or_else(|| CliError::new("Validator account data too small"))?;
        validator_commission_percentages.push(commission);
        // On the chain, the validator's performance is stored in a separate
        // account list, and it is written down in "first come, first serve" order.
        // But here in the CLI, we join the two lists by validator pubkey,
        // so that the two lists have the same indices.
        let perf = available_perfs
            .entries
            .iter()
            .find(|perf| &perf.validator_vote_account_address == validator.pubkey());
        validator_perfs.push(perf.cloned());
    }
    let validators = validators
        .into_iter()
        .zip(validator_identities.into_iter())
        .zip(validator_infos.into_iter())
        .zip(validator_perfs.into_iter())
        .zip(validator_commission_percentages.into_iter())
        .map(
            |((((v, identity), info), perf), commission)| RichValidator {
                active: v.is_active(),
                vote_account_address: v.pubkey().to_owned(),
                stake_seeds: v.stake_seeds,
                unstake_seeds: v.unstake_seeds,
                stake_accounts_balance: v.stake_accounts_balance,
                unstake_accounts_balance: v.unstake_accounts_balance,
                effective_stake_balance: v.effective_stake_balance,
                identity_account_address: identity,
                info,
                perf,
                commission,
            },
        )
        .collect();

    Ok(ShowSolidoOutput {
        solido_program_id: *opts.solido_program_id(),
        solido_address: *opts.solido_address(),
        solido: lido,
        reserve_account,
        stake_authority,
        mint_authority,
        validators,
        validators_max,
        maintainers,
        maintainers_max,
        reserve_account_balance: Lamports(reserve_account_balance),
    })
}

#[derive(Serialize)]
pub struct ShowSolidoAuthoritiesOutput {
    #[serde(serialize_with = "serialize_b58")]
    pub solido_program_id: Pubkey,

    #[serde(serialize_with = "serialize_b58")]
    pub solido_address: Pubkey,

    #[serde(serialize_with = "serialize_b58")]
    pub reserve_account: Pubkey,

    #[serde(serialize_with = "serialize_b58")]
    pub stake_authority: Pubkey,

    #[serde(serialize_with = "serialize_b58")]
    pub mint_authority: Pubkey,
}

impl fmt::Display for ShowSolidoAuthoritiesOutput {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "Stake authority:            {}", self.stake_authority,)?;
        writeln!(f, "Mint authority:             {}", self.mint_authority)?;
        writeln!(f, "Reserve account:            {}", self.reserve_account)?;
        Ok(())
    }
}

pub fn command_show_solido_authorities(
    opts: &ShowSolidoAuthoritiesOpts,
) -> solido_cli_common::Result<ShowSolidoAuthoritiesOutput> {
    let (reserve_account, _) = find_authority_program_address(
        opts.solido_program_id(),
        opts.solido_address(),
        RESERVE_ACCOUNT,
    );
    let (mint_authority, _) = find_authority_program_address(
        opts.solido_program_id(),
        opts.solido_address(),
        MINT_AUTHORITY,
    );
    let (stake_authority, _) = find_authority_program_address(
        opts.solido_program_id(),
        opts.solido_address(),
        STAKE_AUTHORITY,
    );
    Ok(ShowSolidoAuthoritiesOutput {
        solido_program_id: *opts.solido_program_id(),
        solido_address: *opts.solido_address(),
        reserve_account,
        stake_authority,
        mint_authority,
    })
}

#[derive(Serialize)]
pub struct DepositOutput {
    #[serde(serialize_with = "serialize_b58")]
    pub recipient: Pubkey,

    /// Amount of stSOL we expected to receive based on the exchange rate at the time of the deposit.
    ///
    /// This can differ from the actual amount, when a deposit happens close to
    /// an epoch boundary, and an `UpdateExchangeRate` instruction executed before
    /// our deposit, but after we checked the exchange rate.
    #[serde(rename = "expected_st_lamports")]
    pub expected_st_sol: StLamports,

    /// The difference in stSOL balance before and after our deposit.
    ///
    /// If no other transactions touch the recipient account, then this is the
    /// amount of stSOL we got. However, the stSOL account balance might change
    /// for other reasons than just the deposit, if another transaction touched
    /// the account in the same block.
    #[serde(rename = "st_lamports_balance_increase")]
    pub st_sol_balance_increase: StLamports,

    /// Whether we had to create the associated stSOL account. False if one existed already.
    pub created_associated_st_sol_account: bool,
}

impl fmt::Display for DepositOutput {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.created_associated_st_sol_account {
            writeln!(f, "Created recipient stSOL account, it did not yet exist.")?;
        } else {
            writeln!(f, "Recipient stSOL account existed already before deposit.")?;
        }
        writeln!(f, "Recipient stSOL account: {}", self.recipient)?;
        writeln!(f, "Expected stSOL amount:   {}", self.expected_st_sol)?;
        writeln!(
            f,
            "stSOL balance increase:  {}",
            self.st_sol_balance_increase
        )?;
        Ok(())
    }
}

pub fn command_deposit(
    config: &mut SnapshotClientConfig,
    opts: &DepositOpts,
) -> std::result::Result<DepositOutput, Error> {
    let (recipient, created_recipient) = config.with_snapshot(|config| {
        let solido = config.client.get_solido(opts.solido_address())?;

        let recipient = spl_associated_token_account::get_associated_token_address(
            &config.signer.pubkey(),
            &solido.st_sol_mint,
        );

        if !config.client.account_exists(&recipient)? {
            let instr = spl_associated_token_account::create_associated_token_account(
                &config.signer.pubkey(),
                &config.signer.pubkey(),
                &solido.st_sol_mint,
            );

            config.sign_and_send_transaction(&[instr], &[config.signer])?;

            Ok((recipient, true))
        } else {
            Ok((recipient, false))
        }
    })?;

    let (balance_before, exchange_rate) = config.with_snapshot(|config| {
        let balance_before = config
            .client
            .get_spl_token_balance(&recipient)
            .map(StLamports)?;
        let solido = config.client.get_solido(opts.solido_address())?;
        let reserve =
            solido.get_reserve_account(opts.solido_program_id(), opts.solido_address())?;
        let mint_authority =
            solido.get_mint_authority(opts.solido_program_id(), opts.solido_address())?;

        let instr = lido::instruction::deposit(
            opts.solido_program_id(),
            &lido::instruction::DepositAccountsMeta {
                lido: *opts.solido_address(),
                user: config.signer.pubkey(),
                recipient,
                st_sol_mint: solido.st_sol_mint,
                mint_authority,
                reserve_account: reserve,
            },
            *opts.amount_sol(),
        );

        config.sign_and_send_transaction(&[instr], &[config.signer])?;

        Ok((balance_before, solido.exchange_rate))
    })?;

    let balance_after = config.with_snapshot(|config| {
        config
            .client
            .get_spl_token_balance(&recipient)
            .map(StLamports)
    })?;

    let st_sol_balance_increase = StLamports(balance_after.0.saturating_sub(balance_before.0));
    let expected_st_sol = exchange_rate
        .exchange_sol(*opts.amount_sol())
        // If this is not an `Ok`, the transaction should have failed, but if
        // the transaction did not fail, then we do want to show the output; we
        // don't want the user to think that the deposit failed.
        .unwrap_or(StLamports(0));

    let result = DepositOutput {
        recipient,
        expected_st_sol,
        st_sol_balance_increase,
        created_associated_st_sol_account: created_recipient,
    };
    Ok(result)
}

#[derive(Serialize)]
pub struct WithdrawOutput {
    #[serde(serialize_with = "serialize_b58")]
    pub from_token_address: Pubkey,

    /// Amount of SOL that was withdrawn.
    pub withdrawn_sol: Lamports,

    /// Newly created stake account, where the source stake account will be
    /// split to.
    #[serde(serialize_with = "serialize_b58")]
    pub new_stake_account: Pubkey,
}

impl fmt::Display for WithdrawOutput {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "Withdrawn from:          {}", self.from_token_address)?;
        writeln!(f, "Total SOL withdrawn:     {}", self.withdrawn_sol)?;
        writeln!(f, "New stake account:       {}", self.new_stake_account)?;
        Ok(())
    }
}

pub fn command_withdraw(
    config: &mut SnapshotClientConfig,
    opts: &WithdrawOpts,
) -> std::result::Result<WithdrawOutput, Error> {
    let (st_sol_address, new_stake_account) = config.with_snapshot(|config| {
        let solido = config.client.get_solido(opts.solido_address())?;

        let validators = config
            .client
            .get_account_list::<Validator>(&solido.validator_list)?;

        let st_sol_address = spl_associated_token_account::get_associated_token_address(
            &config.signer.pubkey(),
            &solido.st_sol_mint,
        );

        let stake_authority =
            solido.get_stake_authority(opts.solido_program_id(), opts.solido_address())?;

        // Get heaviest validator.
        let heaviest_validator = get_validator_to_withdraw(&validators).map_err(|err| {
            CliError::with_cause(
                "The instance has no active validators to withdraw from.",
                err,
            )
        })?;

        let (stake_address, _bump_seed) = heaviest_validator.find_stake_account_address(
            opts.solido_program_id(),
            opts.solido_address(),
            heaviest_validator.stake_seeds.begin,
            StakeType::Stake,
        );

        let destination_stake_account = Keypair::new();
        let validator_index = validators
            .position(heaviest_validator.pubkey())
            .ok_or_else(|| CliError::new("Pubkey not found in validator list"))?;

        let instr = lido::instruction::withdraw(
            opts.solido_program_id(),
            &lido::instruction::WithdrawAccountsMetaV2 {
                lido: *opts.solido_address(),
                st_sol_mint: solido.st_sol_mint,
                st_sol_account_owner: config.signer.pubkey(),
                st_sol_account: st_sol_address,
                validator_vote_account: *heaviest_validator.pubkey(),
                source_stake_account: stake_address,
                destination_stake_account: destination_stake_account.pubkey(),
                stake_authority,
                validator_list: solido.validator_list,
            },
            *opts.amount_st_sol(),
            validator_index,
        );
        config.sign_and_send_transaction(&[instr], &[config.signer, &destination_stake_account])?;

        Ok((st_sol_address, destination_stake_account))
    })?;

    let stake_sol = config.with_snapshot(|config| {
        let stake_account = config.client.get_account(&new_stake_account.pubkey())?;
        Ok(Lamports(stake_account.lamports()))
    })?;
    let result = WithdrawOutput {
        from_token_address: st_sol_address,
        withdrawn_sol: stake_sol,
        new_stake_account: new_stake_account.pubkey(),
    };
    Ok(result)
}

#[derive(Serialize)]
pub struct DeactivateIfViolatesOutput {
    // List of validators that exceeded max commission
    entries: Vec<ValidatorViolationInfo>,
    max_commission_percentage: u8,
}

#[derive(Serialize)]
struct ValidatorViolationInfo {
    #[serde(serialize_with = "serialize_b58")]
    pub validator_vote_account: Pubkey,
    pub commission: u8,
}

impl fmt::Display for DeactivateIfViolatesOutput {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(
            f,
            "Maximum validation commission: {}",
            self.max_commission_percentage
        )?;

        for entry in &self.entries {
            writeln!(
                f,
                "Validator vote account: {}, validation commission: {}",
                entry.validator_vote_account, entry.commission
            )?;
        }
        Ok(())
    }
}

/// CLI entry point to curate out the validators that violate the thresholds
pub fn command_deactivate_if_violates(
    config: &mut SnapshotConfig,
    opts: &DeactivateIfViolatesOpts,
) -> solido_cli_common::Result<DeactivateIfViolatesOutput> {
    let solido = config.client.get_solido(opts.solido_address())?;

    let validators = config
        .client
        .get_account_list::<Validator>(&solido.validator_list)?;

    let mut violations = vec![];
    let mut instructions = vec![];
    for validator in validators.entries.iter() {
        let vote_pubkey = validator.pubkey();
        let validator_account = config.client.get_account(vote_pubkey)?;
        let commission = get_vote_account_commission(&validator_account.data)
            .ok()
            .ok_or_else(|| CliError::new("Validator account data too small"))?;

        if !validator.is_active() || commission <= solido.criteria.max_commission {
            continue;
        }

        let instruction = lido::instruction::deactivate_if_violates(
            opts.solido_program_id(),
            &lido::instruction::DeactivateIfViolatesMeta {
                lido: *opts.solido_address(),
                validator_vote_account_to_deactivate: *validator.pubkey(),
                validator_list: solido.validator_list,
                validator_perf_list: solido.validator_perf_list,
            },
        );
        instructions.push(instruction);
        violations.push(ValidatorViolationInfo {
            validator_vote_account: *validator.pubkey(),
            commission,
        });
    }

    let signers: Vec<&dyn Signer> = vec![];
    // Due to the fact that Solana has a limit on number of instructions in a transaction
    // this can fall if there would be a lot of misbehaved validators each
    // exceeding `max_commission_percentage`. But it is a very improbable scenario.
    config.sign_and_send_transaction(&instructions, &signers)?;

    Ok(DeactivateIfViolatesOutput {
        entries: violations,
        max_commission_percentage: solido.criteria.max_commission,
    })
}

/// CLI entry point to mark a validator as subject to removal.
pub fn command_remove_validator(
    config: &mut SnapshotConfig,
    opts: &RemoveValidatorOpts,
) -> solido_cli_common::Result<ProposeInstructionOutput> {
    let solido = config.client.get_solido(opts.solido_address())?;

    let validators = config
        .client
        .get_account_list::<Validator>(&solido.validator_list)?;

    let (multisig_address, _) =
        get_multisig_program_address(opts.multisig_program_id(), opts.multisig_address());

    let instruction = lido::instruction::enqueue_validator_for_removal(
        opts.solido_program_id(),
        &lido::instruction::EnqueueValidatorForRemovalMetaV2 {
            lido: *opts.solido_address(),
            manager: multisig_address,
            validator_vote_account_to_remove: *opts.validator_vote_account(),
            validator_list: solido.validator_list,
        },
        validators
            .position(opts.validator_vote_account())
            .ok_or_else(|| CliError::new("Pubkey not found in validator list"))?,
    );
    propose_instruction(
        config,
        opts.multisig_program_id(),
        *opts.multisig_address(),
        instruction,
    )
}

/// CLI entry point to change the thresholds of curating out the validators
pub fn command_change_criteria(
    config: &mut SnapshotConfig,
    opts: &ChangeCriteriaOpts,
) -> solido_cli_common::Result<ProposeInstructionOutput> {
    let (multisig_address, _) =
        get_multisig_program_address(opts.multisig_program_id(), opts.multisig_address());

    let instruction = lido::instruction::change_criteria(
        opts.solido_program_id(),
        &lido::instruction::ChangeCriteriaMeta {
            lido: *opts.solido_address(),
            manager: multisig_address,
        },
        Criteria {
            max_commission: *opts.max_commission(),
            min_block_production_rate: *opts.min_block_production_rate(),
            min_vote_success_rate: *opts.min_vote_success_rate(),
        },
    );
    propose_instruction(
        config,
        opts.multisig_program_id(),
        *opts.multisig_address(),
        instruction,
    )
}

#[derive(Serialize)]
pub struct CreateV2AccountsOutput {
    /// Account that stores validator list data.
    #[serde(serialize_with = "serialize_b58")]
    validator_list_address: Pubkey,
    /// Account that stores maintainer list data.
    #[serde(serialize_with = "serialize_b58")]
    maintainer_list_address: Pubkey,
    /// Account that will receive developer stSOL fee
    #[serde(serialize_with = "serialize_b58")]
    developer_fee_address: Pubkey,
}

impl fmt::Display for CreateV2AccountsOutput {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "Created new v2 accounts:")?;
        writeln!(
            f,
            "  Validator list account:   {}",
            self.validator_list_address
        )?;
        writeln!(
            f,
            "  Maintainer list account:  {}",
            self.maintainer_list_address
        )?;
        writeln!(
            f,
            "  Developer fee account:    {}",
            self.developer_fee_address
        )?;
        Ok(())
    }
}

/// CLI entry point to create new accounts for Solido v2.
pub fn command_create_v2_accounts(
    config: &mut SnapshotConfig,
    opts: &CreateV2AccountsOpts,
) -> solido_cli_common::Result<CreateV2AccountsOutput> {
    let validator_list_signer = Keypair::new();
    let maintainer_list_signer = Keypair::new();

    let validator_list_size = AccountList::<Validator>::required_bytes(50_000);
    let validator_list_account_balance = config
        .client
        .get_minimum_balance_for_rent_exemption(validator_list_size)?;

    let maintainer_list_size = AccountList::<Maintainer>::required_bytes(5_000);
    let maintainer_list_account_balance = config
        .client
        .get_minimum_balance_for_rent_exemption(maintainer_list_size)?;

    let mut instructions = Vec::new();

    let developer_keypair = push_create_spl_token_account(
        config,
        &mut instructions,
        opts.st_sol_mint(),
        opts.developer_account_owner(),
    )?;

    // Create the account that holds the validator list itself.
    instructions.push(system_instruction::create_account(
        &config.signer.pubkey(),
        &validator_list_signer.pubkey(),
        validator_list_account_balance.0,
        validator_list_size as u64,
        opts.solido_program_id(),
    ));

    // Create the account that holds the maintainer list itself.
    instructions.push(system_instruction::create_account(
        &config.signer.pubkey(),
        &maintainer_list_signer.pubkey(),
        maintainer_list_account_balance.0,
        maintainer_list_size as u64,
        opts.solido_program_id(),
    ));

    config.sign_and_send_transaction(
        &instructions[..],
        &[
            config.signer,
            &validator_list_signer,
            &maintainer_list_signer,
            &developer_keypair,
        ],
    )?;
    Ok(CreateV2AccountsOutput {
        validator_list_address: validator_list_signer.pubkey(),
        maintainer_list_address: maintainer_list_signer.pubkey(),
        developer_fee_address: developer_keypair.pubkey(),
    })
}

/// CLI entry point to update Solido state to V2
pub fn command_migrate_state_to_v2(
    config: &mut SnapshotClientConfig,
    opts: &MigrateStateToV2Opts,
) -> solido_cli_common::Result<ProposeInstructionOutput> {
    let propose_output = config.with_snapshot(|config| {
        let (multisig_address, _) =
            get_multisig_program_address(opts.multisig_program_id(), opts.multisig_address());

        let instruction = lido::instruction::migrate_state_to_v2(
            opts.solido_program_id(),
            RewardDistribution {
                treasury_fee: *opts.treasury_fee_share(),
                developer_fee: *opts.developer_fee_share(),
                st_sol_appreciation: *opts.st_sol_appreciation_share(),
            },
            6_700,
            5_000,
            *opts.max_commission_percentage(),
            &lido::instruction::MigrateStateToV2Meta {
                lido: *opts.solido_address(),
                manager: multisig_address,
                validator_list: *opts.validator_list_address(),
                validator_perf_list: *opts.validator_perf_list_address(),
                maintainer_list: *opts.maintainer_list_address(),
                developer_account: *opts.developer_fee_address(),
            },
        );

        propose_instruction(
            config,
            opts.multisig_program_id(),
            *opts.multisig_address(),
            instruction,
        )
    })?;

    Ok(propose_output)
}
