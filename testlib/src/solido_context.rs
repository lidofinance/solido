// SPDX-FileCopyrightText: 2021 Chorus One AG
// SPDX-License-Identifier: GPL-3.0

//! Holds a test context, which makes it easier to test with a Solido instance set up.

use borsh::BorshSerialize;
use num_traits::cast::FromPrimitive;
use rand::prelude::StdRng;
use rand::SeedableRng;
use solana_program::program_pack::Pack;
use solana_program::rent::Rent;
use solana_program::stake::state::Stake;
use solana_program::system_instruction;
use solana_program::system_program;
use solana_program::{borsh::try_from_slice_unchecked, sysvar};
use solana_program::{clock::Clock, instruction::Instruction};
use solana_program::{instruction::InstructionError, stake_history::StakeHistory};
use solana_program_test::{processor, ProgramTest, ProgramTestBanksClientExt, ProgramTestContext};
use solana_sdk::account::{from_account, Account};
use solana_sdk::account_info::AccountInfo;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{Keypair, Signer};
use solana_sdk::transaction::Transaction;
use solana_sdk::transaction::TransactionError;
use solana_sdk::transport;
use solana_sdk::transport::TransportError;
use solana_vote_program::vote_instruction;
use solana_vote_program::vote_state::{VoteInit, VoteState};

use lido::processor::StakeType;
use lido::stake_account::StakeAccount;
use lido::token::{Lamports, StLamports};
use lido::{error::LidoError, instruction, RESERVE_ACCOUNT, STAKE_AUTHORITY};
use lido::{
    state::{
        AccountList, Criteria, FeeRecipients, Lido, ListEntry, Maintainer, RewardDistribution,
        StakeDeposit, Validator, ValidatorPerf,
    },
    MINT_AUTHORITY,
};

pub struct DeterministicKeypairGen {
    rng: StdRng,
}

impl DeterministicKeypairGen {
    fn new() -> Self {
        let rng = StdRng::seed_from_u64(0);
        DeterministicKeypairGen { rng }
    }
    pub fn new_keypair(&mut self) -> Keypair {
        Keypair::generate(&mut self.rng)
    }
}

#[test]
fn test_deterministic_key() {
    let mut deterministic_keypair = DeterministicKeypairGen::new();
    let kp1 = deterministic_keypair.new_keypair();
    let expected_result: &[u8] = &[
        178, 247, 245, 129, 214, 222, 60, 6, 168, 34, 253, 110, 126, 130, 101, 251, 192, 15, 132,
        1, 105, 106, 91, 220, 52, 245, 166, 210, 255, 63, 146, 47, 237, 208, 246, 222, 52, 42, 30,
        106, 114, 54, 214, 36, 79, 35, 216, 62, 237, 252, 236, 208, 89, 163, 134, 200, 80, 85, 112,
        20, 152, 231, 112, 51,
    ];
    assert_eq!(kp1.to_bytes(), expected_result);
}

// Program id for the Solido program. Only used for tests.
solana_program::declare_id!("So1ido1111111111111111111111111111111111112");

pub struct Context {
    pub deterministic_keypair: DeterministicKeypairGen,
    /// Inner test context that contains the banks client and recent block hash.
    pub context: ProgramTestContext,

    /// A nonce to make similar transactions distinct, incremented after every
    /// `send_transaction`.
    pub nonce: u64,

    // Key pairs for the accounts in the Solido instance.
    pub solido: Keypair,
    pub manager: Keypair,
    pub st_sol_mint: Pubkey,
    pub maintainer: Option<Keypair>,
    pub validator: Option<ValidatorAccounts>,
    pub validator_list: Keypair,
    pub validator_perf_list: Keypair,
    pub maintainer_list: Keypair,

    pub treasury_st_sol_account: Pubkey,
    pub developer_st_sol_account: Pubkey,
    pub reward_distribution: RewardDistribution,

    pub reserve_address: Pubkey,
    pub stake_authority: Pubkey,
    pub mint_authority: Pubkey,

    pub criteria: Criteria,
}

pub struct ValidatorAccounts {
    pub node_account: Keypair,
    pub vote_account: Pubkey,
    pub withdraw_authority: Keypair,
}

/// Sign and send a transaction with a fresh block hash.
///
/// The payer always signs, but additional signers can be passed as well.
///
/// Takes a nonce to ensure that sending the same instruction twice will result
/// in distinct transactions. This function increments the nonce after using it.
pub async fn send_transaction(
    context: &mut ProgramTestContext,
    instructions: &[Instruction],
    additional_signers: Vec<&Keypair>,
) -> transport::Result<()> {
    let instructions_mut = instructions.to_vec();

    // If we try to send exactly the same transaction twice, the second one will
    // not be considered distinct by the runtime, and it will not execute, but
    // instead immediately complete successfully. This is undesirable in tests,
    // sometimes we do want to repeat a transaction, e.g. update the exchange
    // rate twice in the same epoch, and confirm that the second one is rejected.
    // Normally the way to do this in Solana is to wait for a new recent block
    // hash. If the block hash is different, the transactions will be distinct.
    context.last_blockhash = context
        .banks_client
        .get_new_latest_blockhash(&context.last_blockhash)
        .await
        .expect("Failed to get a new blockhash.");

    // Change this to true to enable more verbose test output.
    if false {
        for (i, instruction) in instructions_mut.iter().enumerate() {
            println!(
                "Instruction #{} calls program {}.",
                i, instruction.program_id
            );
            for (j, account) in instruction.accounts.iter().enumerate() {
                println!(
                    "  Account {:2}: [{}{}] {}",
                    j,
                    if account.is_writable { 'W' } else { '-' },
                    if account.is_signer { 'S' } else { '-' },
                    account.pubkey,
                );
            }
        }
    }

    let mut transaction =
        Transaction::new_with_payer(&instructions_mut, Some(&context.payer.pubkey()));

    // Sign with the payer, and additional signers if any.
    let mut signers = additional_signers;
    signers.push(&context.payer);
    transaction.sign(&signers, context.last_blockhash);

    let result = context.banks_client.process_transaction(transaction).await;

    // If the transaction failed, try to be helpful by converting the error code
    // back to a message if possible.
    if let Err(TransportError::TransactionError(TransactionError::InstructionError(
        _,
        InstructionError::Custom(error_code),
    ))) = result
    {
        println!("Transaction failed with InstructionError::Custom.");
        match LidoError::from_u32(error_code) {
            Some(err) => println!(
                "If this error originated from Solido, it was this variant: {:?}",
                err
            ),
            None => println!("This error is not a known Solido error."),
        }
    }

    result
}

#[derive(PartialEq, Debug)]
pub struct SolidoWithLists {
    pub lido: Lido,
    pub validators: AccountList<Validator>,
    pub validator_perfs: AccountList<ValidatorPerf>,
    pub maintainers: AccountList<Maintainer>,
}

impl Context {
    /// Set up a new test context with an initialized Solido instance.
    ///
    /// The instance contains no maintainers yet.
    pub async fn new_empty() -> Context {
        let mut deterministic_keypair = DeterministicKeypairGen::new();
        let manager = deterministic_keypair.new_keypair();
        let solido = deterministic_keypair.new_keypair();
        let validator_list = deterministic_keypair.new_keypair();
        let validator_perf_list = deterministic_keypair.new_keypair();
        let maintainer_list = deterministic_keypair.new_keypair();

        let reward_distribution = RewardDistribution {
            treasury_fee: 3,
            developer_fee: 2,
            st_sol_appreciation: 95,
        };

        let (reserve_address, _) = Pubkey::find_program_address(
            &[&solido.pubkey().to_bytes()[..], RESERVE_ACCOUNT],
            &id(),
        );

        let (stake_authority, _) = Pubkey::find_program_address(
            &[&solido.pubkey().to_bytes()[..], STAKE_AUTHORITY],
            &id(),
        );
        let (mint_authority, _) =
            Pubkey::find_program_address(&[&solido.pubkey().to_bytes()[..], MINT_AUTHORITY], &id());

        let mut program_test = ProgramTest::default();
        // Note: the program name *must* match the name of the .so file that contains
        // the program. If it does not, then it will still partially work, but we get
        // weird errors about resizing accounts.
        program_test.add_program(
            "lido",
            crate::solido_context::id(),
            processor!(lido::processor::process),
        );

        let mut result = Self {
            context: program_test.start_with_context().await,
            nonce: 0,
            manager,
            solido,
            validator_list,
            validator_perf_list,
            maintainer_list,
            st_sol_mint: Pubkey::default(),
            maintainer: None,
            validator: None,
            treasury_st_sol_account: Pubkey::default(),
            developer_st_sol_account: Pubkey::default(),
            reward_distribution,
            reserve_address,
            stake_authority,
            mint_authority,
            deterministic_keypair,
            criteria: Criteria::new(5, 0, 0),
        };

        result.st_sol_mint = result.create_mint(result.mint_authority).await;

        let treasury_owner = result.deterministic_keypair.new_keypair();
        result.treasury_st_sol_account =
            result.create_st_sol_account(treasury_owner.pubkey()).await;

        let developer_owner = result.deterministic_keypair.new_keypair();
        result.developer_st_sol_account =
            result.create_st_sol_account(developer_owner.pubkey()).await;

        let max_validators = 10_000;
        let max_maintainers = 10_000;
        let solido_size = Lido::calculate_size();
        let rent = result.context.banks_client.get_rent().await.unwrap();
        let rent_solido = rent.minimum_balance(solido_size);

        let rent_reserve = rent.minimum_balance(0);
        let validator_list_size = AccountList::<Validator>::required_bytes(max_validators);
        let validator_perf_list_size = AccountList::<ValidatorPerf>::required_bytes(max_validators);
        let rent_validator_perf_list = rent.minimum_balance(validator_perf_list_size);
        let rent_validator_list = rent.minimum_balance(validator_list_size);

        let maintainer_list_size = AccountList::<Maintainer>::required_bytes(max_maintainers);
        let rent_maintainer_list = rent.minimum_balance(maintainer_list_size);

        result
            .fund(result.reserve_address, Lamports(rent_reserve))
            .await;

        let payer = result.context.payer.pubkey();
        send_transaction(
            &mut result.context,
            &[
                system_instruction::create_account(
                    &payer,
                    &result.solido.pubkey(),
                    rent_solido,
                    solido_size as u64,
                    &id(),
                ),
                system_instruction::create_account(
                    &payer,
                    &result.validator_list.pubkey(),
                    rent_validator_list,
                    validator_list_size as u64,
                    &id(),
                ),
                system_instruction::create_account(
                    &payer,
                    &result.validator_perf_list.pubkey(),
                    rent_validator_perf_list,
                    validator_perf_list_size as u64,
                    &id(),
                ),
                system_instruction::create_account(
                    &payer,
                    &result.maintainer_list.pubkey(),
                    rent_maintainer_list,
                    maintainer_list_size as u64,
                    &id(),
                ),
                instruction::initialize(
                    &id(),
                    result.reward_distribution.clone(),
                    result.criteria.clone(),
                    max_validators,
                    max_maintainers,
                    &instruction::InitializeAccountsMeta {
                        lido: result.solido.pubkey(),
                        manager: result.manager.pubkey(),
                        st_sol_mint: result.st_sol_mint,
                        treasury_account: result.treasury_st_sol_account,
                        developer_account: result.developer_st_sol_account,
                        reserve_account: result.reserve_address,
                        validator_list: result.validator_list.pubkey(),
                        validator_perf_list: result.validator_perf_list.pubkey(),
                        maintainer_list: result.maintainer_list.pubkey(),
                    },
                ),
            ],
            vec![
                &result.solido,
                &result.validator_list,
                &result.validator_perf_list,
                &result.maintainer_list,
            ],
        )
        .await
        .expect("Failed to initialize Solido instance.");

        result
    }

    /// Set up a new test context, where the Solido instance has a single maintainer.
    pub async fn new_with_maintainer() -> Context {
        let mut result = Context::new_empty().await;
        result.maintainer = Some(result.add_maintainer().await);
        result
    }

    /// Set up a new test context, where the Solido instance has a single maintainer and single validator.
    pub async fn new_with_maintainer_and_validator() -> Context {
        let mut result = Context::new_with_maintainer().await;
        result.validator = Some(result.add_validator().await);
        result
    }

    /// Set up a new test context, where the Solido instance has a single maintainer, one
    /// validator. Deposits 20 Sol and stake 2 accounts with 10 Sol each.
    pub async fn new_with_two_stake_accounts() -> (Context, Vec<Pubkey>) {
        let mut result = Context::new_with_maintainer().await;
        let validator = result.add_validator().await;

        result.deposit(Lamports(20_000_000_000)).await;
        let mut stake_accounts = Vec::new();
        for _ in 0..2 {
            let stake_account = result
                .stake_deposit(
                    validator.vote_account,
                    StakeDeposit::Append,
                    Lamports(10_000_000_000),
                )
                .await;

            stake_accounts.push(stake_account);
        }
        result.validator = Some(validator);
        (result, stake_accounts)
    }

    /// Send a memo transaction. This can be used for printf debugging, because
    /// the memo will show up in the test logs, between the other transaction
    /// logs.
    pub async fn memo(&mut self, message: &str) {
        let memo_instr = spl_memo::build_memo(message.as_bytes(), &[]);
        send_transaction(&mut self.context, &[memo_instr], vec![])
            .await
            .expect("Failed to send memo transaction.")
    }

    /// Warp to the given epoch after the first normal slot.
    ///
    /// Note, the epoch number here is not the same as the epoch number of the
    /// clock sysvar; we start counting from the first "normal" slot.
    pub fn advance_to_normal_epoch(&mut self, epoch: u64) {
        let epoch_schedule = self.context.genesis_config().epoch_schedule;
        let start_slot = epoch_schedule.first_normal_slot;
        let warp_slot = start_slot + epoch * epoch_schedule.slots_per_epoch;
        self.context
            .warp_to_slot(warp_slot)
            .expect("Failed to warp to epoch.");
    }

    /// Initialize a new SPL token mint, return its instance address.
    pub async fn create_mint(&mut self, mint_authority: Pubkey) -> Pubkey {
        let mint = self.deterministic_keypair.new_keypair();

        let rent = self.context.banks_client.get_rent().await.unwrap();
        let mint_rent = rent.minimum_balance(spl_token::state::Mint::LEN);

        let payer = self.context.payer.pubkey();
        send_transaction(
            &mut self.context,
            &[
                system_instruction::create_account(
                    &payer,
                    &mint.pubkey(),
                    mint_rent,
                    spl_token::state::Mint::LEN as u64,
                    &spl_token::id(),
                ),
                spl_token::instruction::initialize_mint(
                    &spl_token::id(),
                    &mint.pubkey(),
                    &mint_authority,
                    None,
                    0,
                )
                .unwrap(),
            ],
            vec![&mint],
        )
        .await
        .expect("Failed to create SPL token mint.");

        mint.pubkey()
    }

    /// Create a new SPL token account, return its address.
    pub async fn create_spl_token_account(&mut self, mint: Pubkey, owner: Pubkey) -> Pubkey {
        let rent = self.context.banks_client.get_rent().await.unwrap();
        let account_rent = rent.minimum_balance(spl_token::state::Account::LEN);
        let account = self.deterministic_keypair.new_keypair();

        let payer = self.context.payer.pubkey();
        send_transaction(
            &mut self.context,
            &[
                system_instruction::create_account(
                    &payer,
                    &account.pubkey(),
                    account_rent,
                    spl_token::state::Account::LEN as u64,
                    &spl_token::id(),
                ),
                spl_token::instruction::initialize_account(
                    &spl_token::id(),
                    &account.pubkey(),
                    &mint,
                    &owner,
                )
                .unwrap(),
            ],
            vec![&account],
        )
        .await
        .expect("Failed to create token account.");

        account.pubkey()
    }

    /// Create a new SPL token account holding stSOL, return its address.
    pub async fn create_st_sol_account(&mut self, owner: Pubkey) -> Pubkey {
        self.create_spl_token_account(self.st_sol_mint, owner).await
    }

    /// Create an initialized but undelegated stake account (outside of Solido).
    pub async fn create_stake_account(
        &mut self,
        fund_amount: Lamports,
        authorized_staker_withdrawer: Pubkey,
    ) -> Pubkey {
        use solana_program::stake::instruction as stake;
        use solana_program::stake::state::{Authorized, Lockup};

        let keypair = self.deterministic_keypair.new_keypair();

        let instructions = stake::create_account(
            &self.context.payer.pubkey(),
            &keypair.pubkey(),
            &Authorized {
                staker: authorized_staker_withdrawer,
                withdrawer: authorized_staker_withdrawer,
            },
            &Lockup::default(),
            fund_amount.0,
        );
        send_transaction(&mut self.context, &instructions[..], vec![&keypair])
            .await
            .expect("Failed to initialize stake account.");

        keypair.pubkey()
    }

    /// Delegate a stake account, outside of Solido.
    pub async fn delegate_stake_account(
        &mut self,
        stake_account: Pubkey,
        vote_account: Pubkey,
        authorized_staker: &Keypair,
    ) {
        use solana_program::stake::instruction as stake;
        let instr =
            stake::delegate_stake(&stake_account, &authorized_staker.pubkey(), &vote_account);
        send_transaction(&mut self.context, &[instr], vec![authorized_staker])
            .await
            .expect("Failed to delegate stake.");
    }

    /// Merge two stake accounts, outside of Solido.
    ///
    /// The authorized staker and withdrawer must be the same for both accounts.
    pub async fn merge_stake_accounts(
        &mut self,
        source: Pubkey,
        destination: Pubkey,
        authorized_staker_withdrawer: &Keypair,
    ) {
        use solana_program::stake::instruction as stake;
        let instructions = stake::merge(
            &destination,
            &source,
            &authorized_staker_withdrawer.pubkey(),
        );
        send_transaction(
            &mut self.context,
            &instructions,
            vec![authorized_staker_withdrawer],
        )
        .await
        .expect("Failed to merge stake.");
    }

    /// Deactivate a stake account, outside of Solido.
    pub async fn deactivate_stake_account(
        &mut self,
        stake_account: Pubkey,
        authorized_staker: &Keypair,
    ) {
        use solana_program::stake::instruction as stake;
        let instruction = stake::deactivate_stake(&stake_account, &authorized_staker.pubkey());
        send_transaction(&mut self.context, &[instruction], vec![authorized_staker])
            .await
            .expect("Failed to deactivate stake.");
    }

    /// Create a vote account for the given validator.
    pub async fn create_vote_account(
        &mut self,
        node_key: &Keypair,
        authorized_withdrawer: Pubkey,
        commission: u8,
    ) -> Pubkey {
        let rent = self.context.banks_client.get_rent().await.unwrap();
        let rent_voter = rent.minimum_balance(VoteState::size_of());

        let initial_balance = Lamports(rent.minimum_balance(0));
        let size_bytes = 0;

        let vote_account = self.deterministic_keypair.new_keypair();

        let payer = self.context.payer.pubkey();
        let mut instructions = vec![system_instruction::create_account(
            &payer,
            &node_key.pubkey(),
            initial_balance.0,
            size_bytes,
            &system_program::id(),
        )];

        instructions.extend(vote_instruction::create_account(
            &payer,
            &vote_account.pubkey(),
            &VoteInit {
                node_pubkey: node_key.pubkey(),
                authorized_voter: node_key.pubkey(),
                authorized_withdrawer,
                commission,
            },
            rent_voter,
        ));
        send_transaction(
            &mut self.context,
            &instructions,
            vec![node_key, &vote_account],
        )
        .await
        .expect("Failed to create vote account.");
        vote_account.pubkey()
    }

    /// Create an account with a given owner and size.
    pub async fn create_account(&mut self, owner: &Pubkey, size: usize) -> Keypair {
        let account = self.deterministic_keypair.new_keypair();
        let payer = self.context.payer.pubkey();
        let rent = self.get_rent().await;
        let lamports = rent.minimum_balance(size);

        send_transaction(
            &mut self.context,
            &[system_instruction::create_account(
                &payer,
                &account.pubkey(),
                lamports,
                size as u64,
                owner,
            )],
            vec![&account],
        )
        .await
        .expect("Failed to create account.");
        account
    }

    /// Make `amount` appear in the recipient's account by transferring it from the context's funder.
    pub async fn fund(&mut self, recipient: Pubkey, amount: Lamports) {
        // Prevent test authors from shooting themselves in their feet by not
        // allowing to leave an account non-rent-exempt, because such accounts
        // might or might not be gone after this function returns, depending on
        // the current epoch and slot, which is very unexpected.
        let rent = self
            .context
            .banks_client
            .get_rent()
            .await
            .expect("Failed to get rent.");
        let min_balance = Lamports(rent.minimum_balance(0));
        let current_balance = self.get_sol_balance(recipient).await;
        if (current_balance + amount).unwrap() < min_balance {
            panic!(
                "You are trying to fund {} with {}, but that would not make the \
                account rent-exempt, it needs at least {} for that.",
                recipient, amount, min_balance,
            )
        }

        let payer = self.context.payer.pubkey();
        send_transaction(
            &mut self.context,
            &[system_instruction::transfer(&payer, &recipient, amount.0)],
            vec![],
        )
        .await
        .unwrap_or_else(|_| panic!("Failed to transfer {} to {}.", amount, recipient));

        // Sanity check to confirm that the account is still there. It should
        // not have been rent-collected, because we enforced that we made it
        // rent-exempt.
        let balance = self.get_sol_balance(recipient).await;
        assert!(
            balance >= amount,
            "Just funded {} with {} but now the balance is {}.",
            recipient,
            amount,
            balance
        );
    }

    pub async fn try_add_maintainer(&mut self, maintainer: Pubkey) -> transport::Result<()> {
        send_transaction(
            &mut self.context,
            &[lido::instruction::add_maintainer(
                &id(),
                &lido::instruction::AddMaintainerMetaV2 {
                    lido: self.solido.pubkey(),
                    manager: self.manager.pubkey(),
                    maintainer,
                    maintainer_list: self.maintainer_list.pubkey(),
                },
            )],
            vec![&self.manager],
        )
        .await
    }

    /// Create a new key pair and add it as maintainer.
    pub async fn add_maintainer(&mut self) -> Keypair {
        let maintainer = self.deterministic_keypair.new_keypair();
        self.try_add_maintainer(maintainer.pubkey())
            .await
            .expect("Failed to add maintainer.");
        maintainer
    }

    pub async fn try_remove_maintainer(&mut self, maintainer: Pubkey) -> transport::Result<()> {
        let solido = self.get_solido().await;
        let maintainer_index = solido.maintainers.position(&maintainer).unwrap();
        send_transaction(
            &mut self.context,
            &[lido::instruction::remove_maintainer(
                &id(),
                &lido::instruction::RemoveMaintainerMetaV2 {
                    lido: self.solido.pubkey(),
                    manager: self.manager.pubkey(),
                    maintainer,
                    maintainer_list: self.maintainer_list.pubkey(),
                },
                maintainer_index,
            )],
            vec![&self.manager],
        )
        .await
    }

    pub async fn try_add_validator(
        &mut self,
        accounts: &ValidatorAccounts,
    ) -> transport::Result<()> {
        send_transaction(
            &mut self.context,
            &[lido::instruction::add_validator(
                &id(),
                &lido::instruction::AddValidatorMetaV2 {
                    lido: self.solido.pubkey(),
                    manager: self.manager.pubkey(),
                    validator_vote_account: accounts.vote_account,
                    validator_list: self.validator_list.pubkey(),
                },
            )],
            vec![&self.manager],
        )
        .await
    }

    /// Create a new key pair and add it as maintainer.
    pub async fn add_validator(&mut self) -> ValidatorAccounts {
        let node_account = self.deterministic_keypair.new_keypair();
        let withdraw_authority = self.deterministic_keypair.new_keypair();
        let vote_account = self
            .create_vote_account(
                &node_account,
                withdraw_authority.pubkey(),
                self.criteria.max_commission,
            )
            .await;

        let accounts = ValidatorAccounts {
            node_account,
            vote_account,
            withdraw_authority,
        };

        self.try_add_validator(&accounts)
            .await
            .expect("Failed to add validator.");

        accounts
    }

    pub async fn deactivate_validator(&mut self, vote_account: Pubkey) {
        let solido = self.get_solido().await;
        let validator_index = solido.validators.position(&vote_account).unwrap();
        send_transaction(
            &mut self.context,
            &[lido::instruction::deactivate_validator(
                &id(),
                &lido::instruction::DeactivateValidatorMetaV2 {
                    lido: self.solido.pubkey(),
                    manager: self.manager.pubkey(),
                    validator_vote_account_to_deactivate: vote_account,
                    validator_list: self.validator_list.pubkey(),
                },
                validator_index,
            )],
            vec![&self.manager],
        )
        .await
        .expect("Failed to deactivate validator.");
    }

    pub async fn try_remove_validator(&mut self, vote_account: Pubkey) -> transport::Result<()> {
        let solido = self.get_solido().await;
        let validator_index = solido.validators.position(&vote_account).unwrap();
        send_transaction(
            &mut self.context,
            &[lido::instruction::remove_validator(
                &id(),
                &lido::instruction::RemoveValidatorMetaV2 {
                    lido: self.solido.pubkey(),
                    validator_vote_account_to_remove: vote_account,
                    validator_list: self.validator_list.pubkey(),
                },
                validator_index,
            )],
            vec![],
        )
        .await
    }

    pub async fn enqueue_validator_for_removal(&mut self, vote_account: Pubkey) {
        let solido = self.get_solido().await;
        let validator_index = solido.validators.position(&vote_account).unwrap();
        send_transaction(
            &mut self.context,
            &[lido::instruction::enqueue_validator_for_removal(
                &id(),
                &lido::instruction::EnqueueValidatorForRemovalMetaV2 {
                    lido: self.solido.pubkey(),
                    manager: self.manager.pubkey(),
                    validator_vote_account_to_remove: vote_account,
                    validator_list: self.validator_list.pubkey(),
                },
                validator_index,
            )],
            vec![&self.manager],
        )
        .await
        .expect("Failed to deactivate validator.");
    }

    pub async fn try_enqueue_validator_for_removal(
        &mut self,
        vote_account: Pubkey,
    ) -> transport::Result<()> {
        let solido = self.get_solido().await;
        let validator_index = solido.validators.position(&vote_account).unwrap();
        send_transaction(
            &mut self.context,
            &[lido::instruction::remove_validator(
                &id(),
                &lido::instruction::RemoveValidatorMetaV2 {
                    lido: self.solido.pubkey(),
                    validator_vote_account_to_remove: vote_account,
                    validator_list: self.validator_list.pubkey(),
                },
                validator_index,
            )],
            vec![],
        )
        .await
    }

    /// Create a new account, deposit from it, and return the resulting owner and stSOL account.
    pub async fn try_deposit(&mut self, amount: Lamports) -> transport::Result<(Keypair, Pubkey)> {
        // Create a new user who is going to do the deposit. The user's account
        // will hold the SOL to deposit, and it will also be the owner of the
        // stSOL account that holds the proceeds.
        let user = self.deterministic_keypair.new_keypair();
        let recipient = self.create_st_sol_account(user.pubkey()).await;

        // Fund the user account, so the user can deposit that into Solido.
        self.fund(user.pubkey(), amount).await;

        send_transaction(
            &mut self.context,
            &[instruction::deposit(
                &id(),
                &instruction::DepositAccountsMeta {
                    lido: self.solido.pubkey(),
                    user: user.pubkey(),
                    recipient,
                    st_sol_mint: self.st_sol_mint,
                    reserve_account: self.reserve_address,
                    mint_authority: self.mint_authority,
                },
                amount,
            )],
            vec![&user],
        )
        .await?;

        Ok((user, recipient))
    }

    pub async fn deposit(&mut self, amount: Lamports) -> (Keypair, Pubkey) {
        self.try_deposit(amount)
            .await
            .expect("Failed to call Deposit on Solido instance.")
    }

    /// Withdraw from the given validator and stake account.
    pub async fn try_withdraw(
        &mut self,
        user: &Keypair,
        st_sol_account: Pubkey,
        amount: StLamports,
        validator_vote_account: Pubkey,
        source_stake_account: Pubkey,
    ) -> transport::Result<Pubkey> {
        // Where the new stake will live.
        let new_stake = self.deterministic_keypair.new_keypair();

        let solido = self.get_solido().await;
        let validator_index = solido.validators.position(&validator_vote_account).unwrap();
        send_transaction(
            &mut self.context,
            &[instruction::withdraw(
                &id(),
                &instruction::WithdrawAccountsMetaV2 {
                    lido: self.solido.pubkey(),
                    st_sol_mint: self.st_sol_mint,
                    st_sol_account_owner: user.pubkey(),
                    st_sol_account,
                    validator_vote_account,
                    source_stake_account,
                    destination_stake_account: new_stake.pubkey(),
                    stake_authority: self.stake_authority,
                    validator_list: self.validator_list.pubkey(),
                },
                amount,
                validator_index,
            )],
            vec![user, &new_stake],
        )
        .await?;
        Ok(new_stake.pubkey())
    }

    /// Withdraw from the given validator and vote account.
    pub async fn withdraw(
        &mut self,
        user: &Keypair,
        st_sol_account: Pubkey,
        amount: StLamports,
        validator_vote_account: Pubkey,
        source_stake_account: Pubkey,
    ) -> Pubkey {
        self.try_withdraw(
            user,
            st_sol_account,
            amount,
            validator_vote_account,
            source_stake_account,
        )
        .await
        .expect("Failed to call Withdraw on Solido instance.")
    }

    /// Stake the given amount to the given validator, return the resulting stake account.
    pub async fn try_stake_deposit(
        &mut self,
        validator_vote_account: Pubkey,
        approach: StakeDeposit,
        amount: Lamports,
    ) -> transport::Result<Pubkey> {
        let solido = self.get_solido().await;

        let validator = solido
            .validators
            .find(&validator_vote_account)
            .expect("Trying to stake with a non-member validator.");

        let validator_index = solido.validators.position(&validator_vote_account).unwrap();
        let (stake_account_end, stake_account_merge_into) = match approach {
            StakeDeposit::Append => {
                let (stake_account_end, _) = validator.find_stake_account_address(
                    &id(),
                    &self.solido.pubkey(),
                    validator.stake_seeds.end,
                    StakeType::Stake,
                );
                (stake_account_end, stake_account_end)
            }
            StakeDeposit::Merge => {
                let (stake_account_end, _) = validator.find_temporary_stake_account_address(
                    &id(),
                    &self.solido.pubkey(),
                    validator.stake_seeds.end,
                    self.get_clock().await.epoch,
                );

                let (stake_account_merge_into, _) = validator.find_stake_account_address(
                    &id(),
                    &self.solido.pubkey(),
                    // We do a wrapping sub here, so we can call stake-merge initially,
                    // when end is 0, such that the account to merge into is not the
                    // same as the end account.
                    validator.stake_seeds.end.wrapping_sub(1),
                    StakeType::Stake,
                );
                (stake_account_end, stake_account_merge_into)
            }
        };

        let maintainer = self
            .maintainer
            .as_ref()
            .expect("Must have maintainer to call StakeDeposit.");

        let maintainer_index = solido.maintainers.position(&maintainer.pubkey()).unwrap();
        send_transaction(
            &mut self.context,
            &[instruction::stake_deposit(
                &id(),
                &instruction::StakeDepositAccountsMetaV2 {
                    lido: self.solido.pubkey(),
                    maintainer: maintainer.pubkey(),
                    validator_vote_account,
                    reserve: self.reserve_address,
                    stake_account_merge_into,
                    stake_account_end,
                    stake_authority: self.stake_authority,
                    validator_list: self.validator_list.pubkey(),
                    maintainer_list: self.maintainer_list.pubkey(),
                },
                amount,
                validator_index,
                maintainer_index,
            )],
            vec![maintainer],
        )
        .await?;

        Ok(stake_account_end)
    }

    /// Stake the given amount to the given validator, return the resulting stake account.
    pub async fn stake_deposit(
        &mut self,
        validator_vote_account: Pubkey,
        approach: StakeDeposit,
        amount: Lamports,
    ) -> Pubkey {
        self.try_stake_deposit(validator_vote_account, approach, amount)
            .await
            .expect("Failed to call StakeDeposit on Solido instance.")
    }

    /// Try to unstake from the validator.
    pub async fn try_unstake(
        &mut self,
        validator_vote_account: Pubkey,
        amount: Lamports,
    ) -> transport::Result<()> {
        // Where the new stake will live.
        let solido = self.get_solido().await;
        let validator = solido.validators.find(&validator_vote_account).unwrap();

        let (source_stake_account, _) = validator.find_stake_account_address(
            &id(),
            &self.solido.pubkey(),
            validator.stake_seeds.begin,
            StakeType::Stake,
        );
        let (destination_unstake_account, _) = validator.find_stake_account_address(
            &id(),
            &self.solido.pubkey(),
            validator.unstake_seeds.end,
            StakeType::Unstake,
        );

        let validator_index = solido.validators.position(&validator_vote_account).unwrap();
        let maintainer = self.maintainer.as_ref().unwrap();
        let maintainer_index = solido.maintainers.position(&maintainer.pubkey()).unwrap();
        send_transaction(
            &mut self.context,
            &[instruction::unstake(
                &id(),
                &instruction::UnstakeAccountsMetaV2 {
                    lido: self.solido.pubkey(),
                    validator_vote_account,
                    source_stake_account,
                    destination_unstake_account,
                    stake_authority: self.stake_authority,
                    maintainer: maintainer.pubkey(),
                    validator_list: self.validator_list.pubkey(),
                    maintainer_list: self.maintainer_list.pubkey(),
                },
                amount,
                validator_index,
                maintainer_index,
            )],
            vec![self.maintainer.as_ref().unwrap()],
        )
        .await?;

        Ok(())
    }

    /// Unstake from the validator.
    pub async fn unstake(&mut self, validator_vote_account: Pubkey, amount: Lamports) {
        self.try_unstake(validator_vote_account, amount)
            .await
            .expect("Failed to call Unstake on Solido instance.");
    }

    pub async fn try_change_reward_distribution(
        &mut self,
        new_reward_distribution: &RewardDistribution,
        new_fee_recipients: &FeeRecipients,
    ) -> transport::Result<()> {
        send_transaction(
            &mut self.context,
            &[instruction::change_reward_distribution(
                &id(),
                new_reward_distribution.clone(),
                &instruction::ChangeRewardDistributionMeta {
                    lido: self.solido.pubkey(),
                    manager: self.manager.pubkey(),
                    treasury_account: new_fee_recipients.treasury_account,
                    developer_account: new_fee_recipients.developer_account,
                },
            )],
            vec![&self.manager],
        )
        .await
    }

    pub async fn try_update_exchange_rate(&mut self) -> transport::Result<()> {
        send_transaction(
            &mut self.context,
            &[instruction::update_exchange_rate(
                &id(),
                &instruction::UpdateExchangeRateAccountsMetaV2 {
                    lido: self.solido.pubkey(),
                    reserve: self.reserve_address,
                    st_sol_mint: self.st_sol_mint,
                    validator_list: self.validator_list.pubkey(),
                },
            )],
            vec![],
        )
        .await
    }

    pub async fn update_exchange_rate(&mut self) {
        self.try_update_exchange_rate()
            .await
            .expect("Failed to update exchange rate.");
    }

    /// Merge two accounts of a given validator.
    ///
    /// Returns the address that stake was merged into.
    pub async fn try_merge_stake(
        &mut self,
        validator: &Validator,
        from_seed: u64,
        to_seed: u64,
    ) -> transport::Result<Pubkey> {
        let (from_stake_account, _) = validator.find_stake_account_address(
            &id(),
            &self.solido.pubkey(),
            from_seed,
            StakeType::Stake,
        );

        let (to_stake_account, _) = validator.find_stake_account_address(
            &id(),
            &self.solido.pubkey(),
            to_seed,
            StakeType::Stake,
        );

        let solido = self.get_solido().await;
        let validator_index = solido.validators.position(validator.pubkey()).unwrap();
        send_transaction(
            &mut self.context,
            &[instruction::merge_stake(
                &id(),
                &instruction::MergeStakeMetaV2 {
                    lido: self.solido.pubkey(),
                    validator_vote_account: *validator.pubkey(),
                    stake_authority: self.stake_authority,
                    from_stake: from_stake_account,
                    to_stake: to_stake_account,
                    validator_list: self.validator_list.pubkey(),
                },
                validator_index,
            )],
            vec![],
        )
        .await?;

        Ok(to_stake_account)
    }

    /// Merge two accounts of a given validator.
    pub async fn merge_stake(
        &mut self,
        validator: &Validator,
        from_seed: u64,
        to_seed: u64,
    ) -> Pubkey {
        self.try_merge_stake(validator, from_seed, to_seed)
            .await
            .expect("Failed to call MergeStake on Solido instance.")
    }

    /// Observe the new validator balance and write it to the state,
    /// distribute any rewards received.
    pub async fn try_update_stake_account_balance(
        &mut self,
        validator_vote_account: Pubkey,
    ) -> transport::Result<()> {
        let solido = self.get_solido().await;
        let validator = solido.validators.find(&validator_vote_account).unwrap();

        let mut stake_account_addrs: Vec<Pubkey> = Vec::new();

        stake_account_addrs.extend(validator.stake_seeds.into_iter().map(|seed| {
            validator
                .find_stake_account_address(&id(), &self.solido.pubkey(), seed, StakeType::Stake)
                .0
        }));
        stake_account_addrs.extend(validator.unstake_seeds.into_iter().map(|seed| {
            validator
                .find_stake_account_address(&id(), &self.solido.pubkey(), seed, StakeType::Unstake)
                .0
        }));

        let validator_index = solido.validators.position(&validator_vote_account).unwrap();

        send_transaction(
            &mut self.context,
            &[instruction::update_stake_account_balance(
                &id(),
                &instruction::UpdateStakeAccountBalanceMeta {
                    lido: self.solido.pubkey(),
                    validator_vote_account,
                    stake_accounts: stake_account_addrs,
                    reserve: self.reserve_address,
                    stake_authority: self.stake_authority,
                    st_sol_mint: self.st_sol_mint,
                    mint_authority: self.mint_authority,
                    treasury_st_sol_account: self.treasury_st_sol_account,
                    developer_st_sol_account: self.developer_st_sol_account,
                    validator_list: self.validator_list.pubkey(),
                },
                validator_index,
            )],
            vec![],
        )
        .await
    }

    pub async fn update_stake_account_balance(&mut self, validator_vote_account: Pubkey) {
        self.try_update_stake_account_balance(validator_vote_account)
            .await
            .expect("Failed to withdraw inactive stake.");
    }

    /// Update the commission in the performance readings for the given validator.
    pub async fn try_update_onchain_validator_perf(
        &mut self,
        validator_vote_account: Pubkey,
    ) -> transport::Result<()> {
        send_transaction(
            &mut self.context,
            &[instruction::update_onchain_validator_perf(
                &id(),
                &instruction::UpdateOnchainValidatorPerfAccountsMeta {
                    lido: self.solido.pubkey(),
                    validator_vote_account_to_update: validator_vote_account,
                    validator_list: self.validator_list.pubkey(),
                    validator_perf_list: self.validator_perf_list.pubkey(),
                },
            )],
            vec![],
        )
        .await
    }

    pub async fn update_onchain_validator_perf_commission(
        &mut self,
        validator_vote_account: Pubkey,
    ) {
        self.try_update_onchain_validator_perf(validator_vote_account)
            .await
            .expect("Validator performance metrics could always be updated");
    }

    /// Update the perf account for the given validator with the given reading.
    pub async fn try_update_offchain_validator_perf(
        &mut self,
        validator_vote_account: Pubkey,
        new_block_production_rate: u64,
        new_vote_success_rate: u64,
    ) -> transport::Result<()> {
        send_transaction(
            &mut self.context,
            &[instruction::update_offchain_validator_perf(
                &id(),
                new_block_production_rate,
                new_vote_success_rate,
                &instruction::UpdateOffchainValidatorPerfAccountsMeta {
                    lido: self.solido.pubkey(),
                    validator_vote_account_to_update: validator_vote_account,
                    validator_list: self.validator_list.pubkey(),
                    validator_perf_list: self.validator_perf_list.pubkey(),
                },
            )],
            vec![],
        )
        .await
    }

    pub async fn update_offchain_validator_perf(
        &mut self,
        validator_vote_account: Pubkey,
        new_block_production_rate: u64,
        new_vote_success_rate: u64,
    ) {
        self.try_update_offchain_validator_perf(
            validator_vote_account,
            new_block_production_rate,
            new_vote_success_rate,
        )
        .await
        .expect("Validator performance metrics could always be updated");
    }

    pub async fn try_get_account(&mut self, address: Pubkey) -> Option<Account> {
        self.context
            .banks_client
            .get_account(address)
            .await
            .expect("Failed to get account, why does this happen in tests?")
    }

    pub async fn try_get_sol_balance(&mut self, address: Pubkey) -> Option<Lamports> {
        self.context
            .banks_client
            .get_balance(address)
            .await
            .ok()
            .map(Lamports)
    }

    pub async fn get_account(&mut self, address: Pubkey) -> Account {
        self.try_get_account(address)
            .await
            .unwrap_or_else(|| panic!("Account {} does not exist.", address))
    }

    pub async fn get_account_list<T>(&mut self, address: Pubkey) -> Option<AccountList<T>>
    where
        T: ListEntry + Clone + Default + BorshSerialize,
    {
        let mut list_account = self.get_account(address).await;
        AccountList::from(&mut list_account.data).ok()
    }

    pub async fn get_sol_balance(&mut self, address: Pubkey) -> Lamports {
        self.try_get_sol_balance(address)
            .await
            .unwrap_or_else(|| panic!("Account {} does not exist.", address))
    }

    pub async fn get_st_sol_balance(&mut self, address: Pubkey) -> StLamports {
        let token_account = self.get_account(address).await;
        let account_info: spl_token::state::Account =
            spl_token::state::Account::unpack_from_slice(token_account.data.as_slice()).unwrap();

        assert_eq!(account_info.mint, self.st_sol_mint);

        StLamports(account_info.amount)
    }

    pub async fn transfer_spl_token(
        &mut self,
        source: &Pubkey,
        destination: &Pubkey,
        authority: &Keypair,
        amount: u64,
    ) {
        send_transaction(
            &mut self.context,
            &[spl_token::instruction::transfer(
                &spl_token::id(),
                source,
                destination,
                &authority.pubkey(),
                &[],
                amount,
            )
            .unwrap()],
            vec![authority],
        )
        .await
        .expect("Failed to transfer tokens.");
    }

    pub async fn get_solido(&mut self) -> SolidoWithLists {
        let lido_account = self.get_account(self.solido.pubkey()).await;
        // This returns a Result because it can cause an IO error, but that should
        // not happen in the test environment. (And if it does, then the test just
        // fails.)
        let lido = try_from_slice_unchecked::<Lido>(lido_account.data.as_slice()).unwrap();
        let validators = self
            .get_account_list::<Validator>(lido.validator_list)
            .await
            .unwrap_or_else(|| AccountList::<Validator>::new_default(0));
        let validator_perfs = self
            .get_account_list::<ValidatorPerf>(lido.validator_perf_list)
            .await
            .unwrap_or_else(|| AccountList::<ValidatorPerf>::new_default(0));
        let maintainers = self
            .get_account_list::<Maintainer>(lido.maintainer_list)
            .await
            .unwrap_or_else(|| AccountList::<Maintainer>::new_default(0));

        SolidoWithLists {
            lido,
            validators,
            validator_perfs,
            maintainers,
        }
    }

    pub async fn get_rent(&mut self) -> Rent {
        self.context
            .banks_client
            .get_rent()
            .await
            .expect("Failed to get rent.")
    }
    pub async fn get_clock(&mut self) -> Clock {
        let account = self
            .context
            .banks_client
            .get_account(sysvar::clock::id())
            .await
            .expect("Clock account should exist.")
            .expect("Clock account should exist.");
        from_account(&account).expect("Could not get Clock from account.")
    }
    pub async fn get_stake_history(&mut self) -> StakeHistory {
        let account = self
            .context
            .banks_client
            .get_account(sysvar::stake_history::id())
            .await
            .expect("Stake History account should exist.")
            .expect("Stake History account should exist.");
        from_account(&account).expect("Could not get Stake History from account.")
    }

    pub async fn get_stake_state(&mut self, stake_account: Pubkey) -> Stake {
        let account = self.get_account(stake_account).await;
        lido::stake_account::deserialize_stake_account(&account.data).unwrap()
    }

    pub async fn get_stake_rent_exempt_reserve(&mut self, stake_account: Pubkey) -> Lamports {
        let account = self.get_account(stake_account).await;
        lido::stake_account::deserialize_rent_exempt_reserve(&account.data).unwrap()
    }

    pub async fn get_stake_account_from_seed(
        &mut self,
        validator: &Validator,
        seed: u64,
    ) -> StakeAccount {
        let (stake_address, _) = validator.find_stake_account_address(
            &id(),
            &self.solido.pubkey(),
            seed,
            StakeType::Stake,
        );

        let clock = self.get_clock().await;
        let stake_history = self.get_stake_history().await;
        let stake_balance = self.get_sol_balance(stake_address).await;
        let stake = self.get_stake_state(stake_address).await;
        StakeAccount::from_delegated_account(stake_balance, &stake, &clock, &stake_history, seed)
    }

    pub async fn get_unstake_account_from_seed(
        &mut self,
        validator: &Validator,
        seed: u64,
    ) -> StakeAccount {
        let (stake_address, _) = validator.find_stake_account_address(
            &id(),
            &self.solido.pubkey(),
            seed,
            StakeType::Unstake,
        );

        let clock = self.get_clock().await;
        let stake_history = self.get_stake_history().await;
        let stake_balance = self.get_sol_balance(stake_address).await;
        let stake = self.get_stake_state(stake_address).await;
        StakeAccount::from_delegated_account(stake_balance, &stake, &clock, &stake_history, seed)
    }

    pub async fn get_vote_account(
        &mut self,
        vote_account: Pubkey,
    ) -> Result<VoteState, InstructionError> {
        let vote_acc = self.get_account(vote_account).await;
        VoteState::deserialize(&vote_acc.data)
    }

    pub async fn try_set_max_commission_percentage(
        &mut self,
        max_commission: u8,
    ) -> transport::Result<()> {
        let solido = self.get_solido().await;
        let current_criteria = solido.lido.criteria;

        send_transaction(
            &mut self.context,
            &[lido::instruction::change_criteria(
                &id(),
                &lido::instruction::ChangeCriteriaMeta {
                    lido: self.solido.pubkey(),
                    manager: self.manager.pubkey(),
                },
                Criteria {
                    max_commission,
                    ..current_criteria
                },
            )],
            vec![&self.manager],
        )
        .await
    }

    pub async fn try_change_criteria(&mut self, new_criteria: &Criteria) -> transport::Result<()> {
        send_transaction(
            &mut self.context,
            &[lido::instruction::change_criteria(
                &id(),
                &lido::instruction::ChangeCriteriaMeta {
                    lido: self.solido.pubkey(),
                    manager: self.manager.pubkey(),
                },
                new_criteria.clone(),
            )],
            vec![&self.manager],
        )
        .await
    }

    pub async fn try_deactivate_if_violates(
        &mut self,
        vote_account: Pubkey,
    ) -> transport::Result<()> {
        send_transaction(
            &mut self.context,
            &[lido::instruction::deactivate_if_violates(
                &id(),
                &lido::instruction::DeactivateIfViolatesMeta {
                    lido: self.solido.pubkey(),
                    validator_vote_account_to_deactivate: vote_account,
                    validator_list: self.validator_list.pubkey(),
                    validator_perf_list: self.validator_perf_list.pubkey(),
                },
            )],
            vec![],
        )
        .await
    }

    pub async fn try_reactivate_if_complies(
        &mut self,
        vote_account: Pubkey,
    ) -> transport::Result<()> {
        send_transaction(
            &mut self.context,
            &[lido::instruction::reactivate_if_complies(
                &id(),
                &lido::instruction::ReactivateIfCompliesMeta {
                    lido: self.solido.pubkey(),
                    validator_vote_account_to_reactivate: vote_account,
                    validator_list: self.validator_list.pubkey(),
                    validator_perf_list: self.validator_perf_list.pubkey(),
                },
            )],
            vec![],
        )
        .await
    }

    pub async fn try_close_vote_account(
        &mut self,
        vote_account: &Pubkey,
        withdraw_authority: &Keypair,
    ) -> transport::Result<()> {
        let vote_info = self.get_account(*vote_account).await;

        send_transaction(
            &mut self.context,
            &[solana_vote_program::vote_instruction::withdraw(
                vote_account,
                &withdraw_authority.pubkey(),
                vote_info.lamports,
                &Pubkey::new_unique(),
            )],
            vec![withdraw_authority],
        )
        .await
    }
}

/// Return an `AccountInfo` for the given account, with `is_signer` and `is_writable` set to false.
pub fn get_account_info<'a>(address: &'a Pubkey, account: &'a mut Account) -> AccountInfo<'a> {
    let is_signer = false;
    let is_writable = false;
    let is_executable = false;
    AccountInfo::new(
        address,
        is_signer,
        is_writable,
        &mut account.lamports,
        &mut account.data,
        &account.owner,
        is_executable,
        account.rent_epoch,
    )
}

#[macro_export]
macro_rules! assert_solido_error {
    ($result:expr, $error:expr $(, /* Accept an optional trailing comma. */)?) => {
        // Open a scope so the imports don't clash.
        {
            use solana_program::instruction::InstructionError;
            use solana_sdk::transaction::TransactionError;
            use solana_sdk::transport::TransportError;
            match $result {
                Err(TransportError::TransactionError(TransactionError::InstructionError(
                    _,
                    InstructionError::Custom(error_code),
                ))) => assert_eq!(
                    error_code,
                    $error as u32,
                    "Expected custom error with code for {}, got different code.",
                    stringify!($error)
                ),
                unexpected => panic!(
                    "Expected {} error, not {:?}",
                    stringify!($error),
                    unexpected
                ),
            }
        }
    };
}

/// Like `assert_solido_error`, but instead of testing for a Solido error, it tests
/// for a raw error code. Can be used to test for errors returned by different programs.
#[macro_export]
macro_rules! assert_error_code {
    ($result:expr, $error_code:expr $(, /* Accept an optional trailing comma. */)?) => {
        // Open a scope so the imports don't clash.
        {
            use solana_program::instruction::InstructionError;
            use solana_sdk::transaction::TransactionError;
            use solana_sdk::transport::TransportError;
            match $result {
                Err(TransportError::TransactionError(TransactionError::InstructionError(
                    _,
                    InstructionError::Custom(error_code),
                ))) => assert_eq!(
                    error_code, $error_code as u32,
                    "Custom error has an unexpected error code.",
                ),
                unexpected => panic!(
                    "Expected custom error with code {} error, not {:?}",
                    $error_code, unexpected
                ),
            }
        }
    };
}
