use lido::error::LidoError;
use lido::state::{Criteria, ListEntry};

use solana_program_test::tokio;
use solana_sdk::signature::Keypair;

use testlib::assert_solido_error;
use testlib::solido_context::Context;

#[tokio::test]
async fn test_curate_by_max_commission_percentage() {
    let mut context = Context::new_with_maintainer_and_validator().await;
    context.advance_to_normal_epoch(0);
    let validator = &context.get_solido().await.validators.entries[0];

    // increase max_commission_percentage
    let result = context.try_set_max_commission_percentage(context.criteria.max_commission + 1);
    assert!(result.await.is_ok());

    let solido = context.get_solido().await.lido;
    assert_eq!(
        solido.criteria.max_commission,
        context.criteria.max_commission + 1
    );

    let result = context.try_deactivate_if_violates(*validator.pubkey());
    assert!(result.await.is_ok());

    // check validator is not deactivated
    let validator = &context.get_solido().await.validators.entries[0];
    assert!(validator.is_active());

    // Increase max_commission_percentage above 100%
    assert_solido_error!(
        context.try_set_max_commission_percentage(101).await,
        LidoError::ValidationCommissionOutOfBounds
    );

    // decrease max_commission_percentage
    let result = context.try_set_max_commission_percentage(context.criteria.max_commission - 1);
    assert!(result.await.is_ok());

    let result = context.try_deactivate_if_violates(*validator.pubkey());
    assert!(result.await.is_ok());

    // check validator is deactivated
    let validator = &context.get_solido().await.validators.entries[0];
    assert!(!validator.is_active());
}

#[tokio::test]
async fn test_curate_by_min_block_production_rate() {
    // Given a Solido context and an active validator:
    let mut context = Context::new_with_maintainer_and_validator().await;
    context.advance_to_normal_epoch(0);
    let validator = &context.get_solido().await.validators.entries[0];
    assert!(validator.is_active());

    // When Solido imposes a minimum block production rate:
    let result = context
        .try_change_criteria(&Criteria {
            min_block_production_rate: 99,
            ..context.criteria
        })
        .await;
    assert!(result.is_ok());

    // And when the validator's block production rate for the epoch is observed:
    let result = context
        .try_update_offchain_validator_perf(*validator.pubkey(), 98, 0)
        .await;
    assert!(result.is_ok());

    // And when the validator's block production rate is below the minimum:
    let result = context
        .try_deactivate_if_violates(*validator.pubkey())
        .await;
    assert!(result.is_ok());

    // Then the validators with a lower block production rate are deactivated:
    let validator = &context.get_solido().await.validators.entries[0];
    assert!(!validator.is_active());
}

#[tokio::test]
async fn test_curate_by_min_vote_success_rate() {
    // Given a Solido context and an active validator:
    let mut context = Context::new_with_maintainer_and_validator().await;
    context.advance_to_normal_epoch(0);
    let validator = &context.get_solido().await.validators.entries[0];
    assert!(validator.is_active());

    // When Solido imposes a minimum vote success rate:
    let result = context
        .try_change_criteria(&Criteria {
            min_vote_success_rate: 99,
            ..context.criteria
        })
        .await;
    assert!(result.is_ok());

    // And when the validator's vote success rate for the epoch is observed:
    let result = context
        .try_update_offchain_validator_perf(*validator.pubkey(), 0, 98)
        .await;
    assert!(result.is_ok());

    // And when the validator's vote success rate is below the minimum:
    let result = context
        .try_deactivate_if_violates(*validator.pubkey())
        .await;
    assert!(result.is_ok());

    // Then the validators with a lower vote success rate are deactivated:
    let validator = &context.get_solido().await.validators.entries[0];
    assert!(!validator.is_active());
}

#[tokio::test]
async fn test_update_block_production_rate() {
    // Given a Solido context and an active validator:
    let mut context = Context::new_with_maintainer_and_validator().await;
    context.advance_to_normal_epoch(0);
    let validator = &context.get_solido().await.validators.entries[0];
    assert!(validator.is_active());

    // When an epoch passes, and the validator's block production rate is observed:
    let result = context
        .try_update_offchain_validator_perf(*validator.pubkey(), 98, 0)
        .await;
    assert!(result.is_ok());

    // Then the validator's block production rate is updated:
    let solido = &context.get_solido().await;
    let perf = &solido
        .validator_perfs
        .entries
        .iter()
        .find(|x| x.validator_vote_account_address == *validator.pubkey())
        .unwrap();
    assert!(perf
        .rest
        .as_ref()
        .map_or(false, |x| x.block_production_rate == 98));
}

#[tokio::test]
async fn test_update_vote_success_rate() {
    // Given a Solido context and an active validator:
    let mut context = Context::new_with_maintainer_and_validator().await;
    context.advance_to_normal_epoch(0);
    let validator = &context.get_solido().await.validators.entries[0];
    assert!(validator.is_active());

    // When an epoch passes, and the validator's vote success rate is observed:
    let result = context
        .try_update_offchain_validator_perf(*validator.pubkey(), 0, 98)
        .await;
    assert!(result.is_ok());

    // Then the validator's vote success rate is updated:
    let solido = &context.get_solido().await;
    let perf = &solido
        .validator_perfs
        .entries
        .iter()
        .find(|x| x.validator_vote_account_address == *validator.pubkey())
        .unwrap();
    assert!(perf
        .rest
        .as_ref()
        .map_or(false, |x| x.vote_success_rate == 98));
}

#[tokio::test]
async fn test_perf_updates_at_most_once_per_epoch() {
    // Given a Solido context and an active validator:
    let mut context = Context::new_with_maintainer_and_validator().await;
    context.advance_to_normal_epoch(0);
    let validator = &context.get_solido().await.validators.entries[0];
    assert!(validator.is_active());

    // When the uptime of a validator gets updated:
    let result = context
        .try_update_offchain_validator_perf(*validator.pubkey(), 98, 0)
        .await;
    assert!(result.is_ok());

    // And when the uptime of the same validator gets updated again in the same epoch:
    let result = context
        .try_update_offchain_validator_perf(*validator.pubkey(), 99, 0)
        .await;

    // Then the second update fails:
    assert_solido_error!(result, LidoError::ValidatorPerfAlreadyUpdatedForEpoch);

    // But when the epoch changes:
    context.advance_to_normal_epoch(1);

    // Then the second update succeeds:
    let result = context
        .try_update_offchain_validator_perf(*validator.pubkey(), 99, 0)
        .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_bring_back() {
    // Given a previously deactivated validator:
    let mut context = Context::new_with_maintainer_and_validator().await;
    context.advance_to_normal_epoch(0);
    let validator = &context.get_solido().await.validators.entries[0];

    let result = context
        .try_change_criteria(&Criteria {
            min_block_production_rate: 99,
            ..context.criteria
        })
        .await;
    assert!(result.is_ok());

    let result = context
        .try_update_offchain_validator_perf(*validator.pubkey(), 98, 0)
        .await;
    assert!(result.is_ok());

    let result = context.try_deactivate_if_violates(*validator.pubkey());
    assert!(result.await.is_ok());

    let validator = &context.get_solido().await.validators.entries[0];
    assert!(!validator.is_active());

    // When the epoch passes:
    context.advance_to_normal_epoch(1);

    // And when the validator's performance is back to normal:
    let result = context
        .try_update_offchain_validator_perf(*validator.pubkey(), 101, 0)
        .await;
    assert!(result.is_ok());

    // And when the instruction is issued:
    let result = context.try_reactivate_if_complies(*validator.pubkey());
    assert!(result.await.is_ok());

    // Then the validator is reactivated:
    let validator = &context.get_solido().await.validators.entries[0];
    assert!(validator.is_active());
}

#[tokio::test]
async fn test_close_vote_account() {
    let mut context = Context::new_with_maintainer_and_validator().await;
    let vote_account = context.validator.as_ref().unwrap().vote_account;

    let validator = &context.get_solido().await.validators.entries[0];
    assert!(validator.is_active());

    let keypair_bytes = context
        .validator
        .as_ref()
        .unwrap()
        .withdraw_authority
        .to_bytes();

    let withdraw_authority = Keypair::from_bytes(&keypair_bytes).unwrap();

    let result = context.try_close_vote_account(&vote_account, &withdraw_authority);
    assert!(result.await.is_ok());

    let result = context.try_deactivate_if_violates(*validator.pubkey());
    assert!(result.await.is_ok());

    let validator = &context.get_solido().await.validators.entries[0];
    assert!(!validator.is_active());
}
