//! A collection of node-specific RPC methods.
//! Substrate provides the `sc-rpc` crate, which defines the core RPC layer
//! used by Substrate nodes. This file extends those RPC definitions with
//! capabilities that are specific to this project's runtime configuration.

#![warn(missing_docs)]

use std::sync::Arc;

use jsonrpsee::RpcModule;

use sc_consensus_grandpa::{
	FinalityProofProvider, GrandpaJustificationStream, SharedAuthoritySet, SharedVoterState,
};
use sc_rpc::SubscriptionTaskExecutor;
use sc_transaction_pool_api::TransactionPool;

use sp_api::ProvideRuntimeApi;
use sp_block_builder::BlockBuilder;
use sp_blockchain::{Error as BlockChainError, HeaderBackend, HeaderMetadata};
use sp_runtime::traits::Block as BlockT;

use numen_runtime::{opaque::Block, AccountId, Balance, Nonce};

/// Dependencies for GRANDPA RPC.
pub struct GrandpaDeps<B> {
	/// Voting round info.
	pub shared_voter_state: SharedVoterState,
	/// Authority set info.
	pub shared_authority_set: SharedAuthoritySet<
		<Block as BlockT>::Hash,
		<<Block as BlockT>::Header as sp_runtime::traits::Header>::Number,
	>,
	/// Receives notifications about justification events from GRANDPA.
	pub justification_stream: GrandpaJustificationStream<Block>,
	/// Executor to drive the subscription manager in the GRANDPA RPC handler.
	pub subscription_executor: SubscriptionTaskExecutor,
	/// Finality proof provider.
	pub finality_provider: Arc<FinalityProofProvider<B, Block>>,
}

/// Dependencies for the external mining RPC.
pub struct MiningDeps {
	/// Worker handle exposing the current task and seal submission.
	pub handle: Arc<dyn crate::mining_rpc::ExternalMiner>,
	/// Configured reward address, in SS58.
	pub miner: String,
	/// Active protocol string.
	pub protocol: String,
}

/// Full client dependencies.
pub struct FullDeps<C, P, B> {
	/// The client instance to use.
	pub client: Arc<C>,
	/// Transaction pool instance.
	pub pool: Arc<P>,
	/// GRANDPA-specific dependencies.
	pub grandpa: GrandpaDeps<B>,
	/// External mining dependencies, present when the operator exposes the RPC.
	pub mining: Option<MiningDeps>,
}

/// Instantiate all full RPC extensions.
pub fn create_full<C, P, B>(
	deps: FullDeps<C, P, B>,
) -> Result<RpcModule<()>, Box<dyn std::error::Error + Send + Sync>>
where
	C: ProvideRuntimeApi<Block>,
	C: HeaderBackend<Block> + HeaderMetadata<Block, Error = BlockChainError> + 'static,
	C: Send + Sync + 'static,
	C::Api: substrate_frame_rpc_system::AccountNonceApi<Block, AccountId, Nonce>,
	C::Api: pallet_transaction_payment_rpc::TransactionPaymentRuntimeApi<Block, Balance>,
	C::Api: BlockBuilder<Block>,
	C::Api: poscan_pow::PowVerifyApi<Block>,
	P: TransactionPool<Block = Block, Hash = <Block as BlockT>::Hash> + 'static,
	B: sc_client_api::Backend<Block> + Send + Sync + 'static,
{
	use pallet_transaction_payment_rpc::{TransactionPayment, TransactionPaymentApiServer};
	use sc_consensus_grandpa_rpc::{Grandpa, GrandpaApiServer};
	use substrate_frame_rpc_system::{System, SystemApiServer};

	let mut module = RpcModule::new(());
	let FullDeps { client, pool, grandpa, mining } = deps;
	let GrandpaDeps {
		shared_voter_state,
		shared_authority_set,
		justification_stream,
		subscription_executor,
		finality_provider,
	} = grandpa;

	module.merge(System::new(client.clone(), pool.clone()).into_rpc())?;
	module.merge(TransactionPayment::new(client.clone()).into_rpc())?;
	module.merge(
		Grandpa::new(
			subscription_executor,
			shared_authority_set,
			shared_voter_state,
			justification_stream,
			finality_provider,
		)
		.into_rpc(),
	)?;

	if let Some(MiningDeps { handle, miner, protocol }) = mining {
		use crate::mining_rpc::{Mining, MiningApiServer};
		module.merge(Mining::new(handle, miner, protocol).into_rpc())?;
	}

	// Model export needs only the client, so every node exposes it regardless of
	// whether it mines.
	{
		use crate::object_rpc::{ObjectApiServer, ObjectExport};
		module.merge(ObjectExport::<C, Block>::new(client.clone()).into_rpc())?;
	}

	Ok(module)
}
