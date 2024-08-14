use anyhow::Context;
use pathfinder_common::BlockId;
use pathfinder_executor::{ExecutionState, L1BlobDataAvailability};
use serde_with::serde_as;

use crate::context::RpcContext;
use crate::error::ApplicationError;
use crate::v02::types::request::BroadcastedTransaction;

#[derive(serde::Deserialize, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EstimateFeeInput {
    pub request: Vec<BroadcastedTransaction>,
    pub block_id: BlockId,
}

#[derive(Debug)]
pub enum EstimateFeeError {
    Internal(anyhow::Error),
    Custom(anyhow::Error),
    BlockNotFound,
    ContractNotFound,
    ContractErrorV05 { revert_error: String },
}

impl From<anyhow::Error> for EstimateFeeError {
    fn from(e: anyhow::Error) -> Self {
        Self::Internal(e)
    }
}

impl From<pathfinder_executor::TransactionExecutionError> for EstimateFeeError {
    fn from(value: pathfinder_executor::TransactionExecutionError) -> Self {
        use pathfinder_executor::TransactionExecutionError::*;
        match value {
            ExecutionError {
                transaction_index,
                error,
            } => Self::ContractErrorV05 {
                revert_error: format!(
                    "Execution error at transaction index {}: {}",
                    transaction_index, error
                ),
            },
            Internal(e) => Self::Internal(e),
            Custom(e) => Self::Custom(e),
        }
    }
}

impl From<crate::executor::ExecutionStateError> for EstimateFeeError {
    fn from(error: crate::executor::ExecutionStateError) -> Self {
        use crate::executor::ExecutionStateError::*;
        match error {
            BlockNotFound => Self::BlockNotFound,
            Internal(e) => Self::Internal(e),
        }
    }
}

impl From<EstimateFeeError> for ApplicationError {
    fn from(value: EstimateFeeError) -> Self {
        match value {
            EstimateFeeError::BlockNotFound => ApplicationError::BlockNotFound,
            EstimateFeeError::ContractNotFound => ApplicationError::ContractNotFound,
            EstimateFeeError::ContractErrorV05 { revert_error } => {
                ApplicationError::ContractErrorV05 { revert_error }
            }
            EstimateFeeError::Internal(e) => ApplicationError::Internal(e),
            EstimateFeeError::Custom(e) => ApplicationError::Custom(e),
        }
    }
}

#[serde_as]
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct FeeEstimate {
    #[serde_as(as = "pathfinder_serde::U256AsHexStr")]
    pub gas_consumed: primitive_types::U256,
    #[serde_as(as = "pathfinder_serde::U256AsHexStr")]
    pub gas_price: primitive_types::U256,
    #[serde_as(as = "pathfinder_serde::U256AsHexStr")]
    pub overall_fee: primitive_types::U256,
}

impl From<pathfinder_executor::types::FeeEstimate> for FeeEstimate {
    fn from(value: pathfinder_executor::types::FeeEstimate) -> Self {
        Self {
            gas_consumed: value.gas_consumed,
            gas_price: value.gas_price,
            overall_fee: value.overall_fee,
        }
    }
}

pub async fn estimate_fee(
    context: RpcContext,
    input: EstimateFeeInput,
) -> Result<Vec<FeeEstimate>, EstimateFeeError> {
    let span = tracing::Span::current();

    let result = tokio::task::spawn_blocking(move || {
        let _g = span.enter();
        let mut db = context
            .execution_storage
            .connection()
            .context("Creating database connection")?;
        let db = db.transaction().context("Creating database transaction")?;

        let (header, pending) = match input.block_id {
            BlockId::Pending => {
                let pending = context
                    .pending_data
                    .get(&db)
                    .context("Querying pending data")?;

                (pending.header(), Some(pending.state_update.clone()))
            }
            other => {
                let block_id = other.try_into().expect("Only pending cast should fail");
                let header = db
                    .block_header(block_id)
                    .context("Querying block header")?
                    .ok_or(EstimateFeeError::BlockNotFound)?;

                (header, None)
            }
        };

        let state = ExecutionState::simulation(
            &db,
            context.chain_id,
            header,
            pending,
            L1BlobDataAvailability::Disabled,
            context.config.custom_versioned_constants,
        );

        let transactions = input
            .request
            .into_iter()
            .map(|tx| crate::executor::map_broadcasted_transaction(&tx, context.chain_id))
            .collect::<Result<Vec<_>, _>>()?;

        let result = pathfinder_executor::estimate(
            state,
            transactions,
            false,
            // skip nonce check because it is not necessary for fee estimation
            true,
        )?;

        Ok::<_, EstimateFeeError>(result)
    })
    .await
    .context("Executing transaction")??;

    Ok(result.into_iter().map(Into::into).collect())
}

#[cfg(test)]
pub(crate) mod tests {
    use pathfinder_common::{
        felt,
        BlockHash,
        CallParam,
        ContractAddress,
        Fee,
        TransactionNonce,
        TransactionSignatureElem,
        TransactionVersion,
    };

    use super::*;
    use crate::v02::types::request::BroadcastedInvokeTransaction;

    mod parsing {
        use serde_json::json;

        use super::*;

        fn test_invoke_txn() -> BroadcastedTransaction {
            BroadcastedTransaction::Invoke(BroadcastedInvokeTransaction::V1(
                crate::v02::types::request::BroadcastedInvokeTransactionV1 {
                    version: TransactionVersion::ONE_WITH_QUERY_VERSION,
                    max_fee: Fee(felt!("0x6")),
                    signature: vec![TransactionSignatureElem(felt!("0x7"))],
                    nonce: TransactionNonce(felt!("0x8")),
                    sender_address: ContractAddress::new_or_panic(felt!("0xaaa")),
                    calldata: vec![CallParam(felt!("0xff"))],
                },
            ))
        }

        #[test]
        fn positional_args() {
            let positional = json!([
                [
                    {
                        "type": "INVOKE",
                        "version": "0x100000000000000000000000000000001",
                        "max_fee": "0x6",
                        "signature": [
                            "0x7"
                        ],
                        "nonce": "0x8",
                        "sender_address": "0xaaa",
                        "calldata": [
                            "0xff"
                        ]
                    }
                ],
                { "block_hash": "0xabcde" }
            ]);

            let input = serde_json::from_value::<EstimateFeeInput>(positional).unwrap();
            let expected = EstimateFeeInput {
                request: vec![test_invoke_txn()],
                block_id: BlockId::Hash(BlockHash(felt!("0xabcde"))),
            };
            assert_eq!(input, expected);
        }

        #[test]
        fn named_args() {
            let named_args = json!({
                "request": [
                    {
                        "type": "INVOKE",
                        "version": "0x100000000000000000000000000000001",
                        "max_fee": "0x6",
                        "signature": [
                            "0x7"
                        ],
                        "nonce": "0x8",
                        "sender_address": "0xaaa",
                        "calldata": [
                            "0xff"
                        ]
                    }
                ],
                "block_id": { "block_hash": "0xabcde" }
            });
            let input = serde_json::from_value::<EstimateFeeInput>(named_args).unwrap();
            let expected = EstimateFeeInput {
                request: vec![test_invoke_txn()],
                block_id: BlockId::Hash(BlockHash(felt!("0xabcde"))),
            };
            assert_eq!(input, expected);
        }
    }

    mod in_memory {

        use pathfinder_common::macro_prelude::*;
        use pathfinder_common::{felt, EntryPoint};

        use super::*;
        use crate::v02::types::request::{
            BroadcastedDeclareTransaction,
            BroadcastedDeclareTransactionV2,
            BroadcastedInvokeTransactionV0,
            BroadcastedInvokeTransactionV1,
        };
        use crate::v02::types::{ContractClass, SierraContractClass};

        #[test_log::test(tokio::test)]
        async fn declare_deploy_and_invoke_sierra_class() {
            let (context, last_block_header, account_contract_address, universal_deployer_address) =
                crate::test_setup::test_context().await;

            let sierra_definition =
                include_bytes!("../../../fixtures/contracts/storage_access.json");
            let sierra_hash =
                class_hash!("0544b92d358447cb9e50b65092b7169f931d29e05c1404a2cd08c6fd7e32ba90");
            let casm_hash =
                casm_hash!("0x069032ff71f77284e1a0864a573007108ca5cc08089416af50f03260f5d6d4d8");

            let contract_class: SierraContractClass =
                ContractClass::from_definition_bytes(sierra_definition)
                    .unwrap()
                    .as_sierra()
                    .unwrap();

            assert_eq!(contract_class.class_hash().unwrap().hash(), sierra_hash);

            let max_fee = Fee::default();

            // declare test class
            let declare_transaction = BroadcastedTransaction::Declare(
                BroadcastedDeclareTransaction::V2(BroadcastedDeclareTransactionV2 {
                    version: TransactionVersion::TWO,
                    max_fee,
                    signature: vec![],
                    nonce: TransactionNonce(Default::default()),
                    contract_class,
                    sender_address: account_contract_address,
                    compiled_class_hash: casm_hash,
                }),
            );
            // deploy with unversal deployer contract
            let deploy_transaction = BroadcastedTransaction::Invoke(
                BroadcastedInvokeTransaction::V1(BroadcastedInvokeTransactionV1 {
                    nonce: transaction_nonce!("0x1"),
                    version: TransactionVersion::ONE,
                    max_fee,
                    signature: vec![],
                    sender_address: account_contract_address,
                    calldata: vec![
                        CallParam(*universal_deployer_address.get()),
                        // Entry point selector for the called contract, i.e.
                        // AccountCallArray::selector
                        CallParam(EntryPoint::hashed(b"deployContract").0),
                        // Length of the call data for the called contract, i.e.
                        // AccountCallArray::data_len
                        call_param!("4"),
                        // classHash
                        CallParam(sierra_hash.0),
                        // salt
                        call_param!("0x0"),
                        // unique
                        call_param!("0x0"),
                        // calldata_len
                        call_param!("0x0"),
                    ],
                }),
            );
            // invoke deployed contract
            let invoke_transaction = BroadcastedTransaction::Invoke(
                BroadcastedInvokeTransaction::V1(BroadcastedInvokeTransactionV1 {
                    nonce: transaction_nonce!("0x2"),
                    version: TransactionVersion::ONE,
                    max_fee,
                    signature: vec![],
                    sender_address: account_contract_address,
                    calldata: vec![
                        // address of the deployed test contract
                        CallParam(felt!(
                            "0x012592426632af714f43ccb05536b6044fc3e897fa55288f658731f93590e7e7"
                        )),
                        // Entry point selector for the called contract, i.e.
                        // AccountCallArray::selector
                        CallParam(EntryPoint::hashed(b"get_data").0),
                        // Length of the call data for the called contract, i.e.
                        // AccountCallArray::data_len
                        call_param!("0"),
                    ],
                }),
            );

            // do the same invoke with a v0 transaction
            let invoke_v0_transaction = BroadcastedTransaction::Invoke(
                BroadcastedInvokeTransaction::V0(BroadcastedInvokeTransactionV0 {
                    version: TransactionVersion::ONE,
                    max_fee,
                    signature: vec![],
                    contract_address: contract_address!(
                        "0x012592426632af714f43ccb05536b6044fc3e897fa55288f658731f93590e7e7"
                    ),
                    entry_point_selector: EntryPoint::hashed(b"get_data"),
                    calldata: vec![],
                }),
            );

            let input = EstimateFeeInput {
                request: vec![
                    declare_transaction,
                    deploy_transaction,
                    invoke_transaction,
                    invoke_v0_transaction,
                ],
                block_id: BlockId::Number(last_block_header.number),
            };
            let result = estimate_fee(context, input).await.unwrap();
            let declare_expected = FeeEstimate {
                gas_consumed: 2768.into(),
                gas_price: 1.into(),
                overall_fee: 2768.into(),
            };
            let deploy_expected = FeeEstimate {
                gas_consumed: 3020.into(),
                gas_price: 1.into(),
                overall_fee: 3020.into(),
            };
            let invoke_expected = FeeEstimate {
                gas_consumed: 1674.into(),
                gas_price: 1.into(),
                overall_fee: 1674.into(),
            };
            let invoke_v0_expected = FeeEstimate {
                gas_consumed: 1669.into(),
                gas_price: 1.into(),
                overall_fee: 1669.into(),
            };
            assert_eq!(
                result,
                vec![
                    declare_expected,
                    deploy_expected,
                    invoke_expected,
                    invoke_v0_expected
                ]
            );
        }
    }
}
