//! Service and ServiceFactory implementation. Specialized wrapper over substrate service.

use futures::FutureExt;
use poscan_pow::PoScanAlgorithm;
use sc_client_api::{Backend, BlockBackend};
use sc_consensus::LongestChain;
use sc_consensus_grandpa::{GrandpaBlockImport, LinkHalf, SharedVoterState};
use sc_consensus_pow::PowBlockImport;
use sc_service::{error::Error as ServiceError, Configuration, TaskManager};
use sc_telemetry::{Telemetry, TelemetryWorker};
use sc_transaction_pool_api::OffchainTransactionPoolFactory;
use numen_runtime::{self, apis::RuntimeApi, opaque::Block, AccountId};
use sp_core::{crypto::Ss58Codec, H256, U256};
use std::{sync::{Arc, Mutex}, time::Duration};

use crate::mining_rpc::ExternalMiner;

/// The RPC size limits in [`Configuration`] are in megabytes.
const MEGABYTE: u32 = 1024 * 1024;

/// Full client type.
pub type FullClient = sc_service::TFullClient<
	Block,
	RuntimeApi,
	sc_executor::WasmExecutor<HostFunctions>,
>;
/// Full backend type.
pub type FullBackend = sc_service::TFullBackend<Block>;
type FullSelectChain = LongestChain<FullBackend, Block>;

/// Host functions exposed to the runtime.
///
/// Runtimes built with pallet-evm import `sp_io::storage_proof_size`, which
/// lives outside the substrate default set. A node syncing from genesis cannot
/// execute their blocks without it.
pub type HostFunctions = (
	sp_io::SubstrateHostFunctions,
	cumulus_primitives_proof_size_hostfunction::storage_proof_size::HostFunctions,
);

/// The minimum period of blocks on which justifications will be imported and generated.
const GRANDPA_JUSTIFICATION_PERIOD: u32 = 30;

pub type Service = sc_service::PartialComponents<
	FullClient,
	FullBackend,
	FullSelectChain,
	sc_consensus::DefaultImportQueue<Block>,
	sc_transaction_pool::TransactionPoolHandle<Block, FullClient>,
	(
		GrandpaBlockImport<FullBackend, Block, FullClient, FullSelectChain>,
		LinkHalf<Block, FullClient, FullSelectChain>,
		Option<Telemetry>,
	),
>;

pub fn new_partial(config: &Configuration) -> Result<Service, ServiceError> {
	let telemetry = config
		.telemetry_endpoints
		.clone()
		.filter(|x| !x.is_empty())
		.map(|endpoints| -> Result<_, sc_telemetry::Error> {
			let worker = TelemetryWorker::new(16)?;
			let telemetry = worker.handle().new_telemetry(endpoints);
			Ok((worker, telemetry))
		})
		.transpose()?;

	let executor = sc_service::new_wasm_executor::<HostFunctions>(&config.executor);
	let (client, backend, keystore_container, task_manager) =
		sc_service::new_full_parts::<Block, RuntimeApi, _>(
			config,
			telemetry.as_ref().map(|(_, telemetry)| telemetry.handle()),
			executor,
			vec![],
		)?;
	let client = Arc::new(client);

	let telemetry = telemetry.map(|(worker, telemetry)| {
		task_manager.spawn_handle().spawn("telemetry", None, worker.run());
		telemetry
	});

	let select_chain = LongestChain::new(backend.clone());

	let transaction_pool = Arc::from(
		sc_transaction_pool::Builder::new(
			task_manager.spawn_essential_handle(),
			client.clone(),
			config.role.is_authority().into(),
		)
		.with_options(config.transaction_pool.clone())
		.with_prometheus(config.prometheus_registry())
		.build(),
	);

	let algorithm = PoScanAlgorithm::new(client.clone());

	let (grandpa_block_import, grandpa_link) = sc_consensus_grandpa::block_import(
		client.clone(),
		GRANDPA_JUSTIFICATION_PERIOD,
		&client,
		select_chain.clone(),
		telemetry.as_ref().map(|x| x.handle()),
	)?;

	let pow_block_import = PowBlockImport::new(
		grandpa_block_import.clone(),
		client.clone(),
		algorithm.clone(),
		0u32,
		select_chain.clone(),
		move |_, ()| async { Ok(sp_timestamp::InherentDataProvider::from_system_time()) },
	);

	let import_queue = sc_consensus_pow::import_queue(
		Box::new(pow_block_import),
		Some(Box::new(grandpa_block_import.clone())),
		algorithm,
		&task_manager.spawn_essential_handle(),
		config.prometheus_registry(),
	)?;

	Ok(sc_service::PartialComponents {
		client,
		backend,
		task_manager,
		import_queue,
		keystore_container,
		select_chain,
		transaction_pool,
		other: (grandpa_block_import, grandpa_link, telemetry),
	})
}

/// PoW algorithm in use by the node miner.
type PowAlgo = PoScanAlgorithm<Block, FullClient>;
/// Concrete mining worker handle type.
type PowMiningHandle<L> = sc_consensus_pow::MiningHandle<Block, PowAlgo, L>;

/// Servers the node hands the Ethereum JSON-RPC methods to.
#[derive(Clone)]
pub struct EthRpcConfig {
	/// eth-rpc server.
	pub eth_rpc: String,
	/// Node that still runs Frontier and answers for the blocks before
	/// pallet-revive.
	pub frozen: Option<String>,
}

/// Operator mining configuration. The external mining RPC is always exposed.
/// `node_miner` is the local scan thread count, zero when the node only relays
/// work to external miners.
pub struct MiningConfig {
	/// Reward and seed-bound miner address.
	pub miner: AccountId,
	/// Local scan threads. Zero disables the in-process miner.
	pub node_miner: usize,
}

impl<L> ExternalMiner for PowMiningHandle<L>
where
	L: sc_consensus::JustificationSyncLink<Block> + Send + Sync + 'static,
{
	fn current_task(&self) -> Option<(H256, U256)> {
		self.metadata().map(|m| (m.pre_hash, m.difficulty))
	}

	fn submit_seal(&self, pre_hash: H256, seal: Vec<u8>) -> bool {
		futures::executor::block_on(self.submit(pre_hash, seal))
	}

	fn task_changes(&self) -> tokio::sync::watch::Receiver<u64> {
		self.subscribe()
	}
}

struct BestSolution {
	pre_hash: H256,
	nonce: U256,
	work: H256,
	/// Highest difficulty this work satisfies, equal to U256::MAX divided by the work.
	max_difficulty: U256,
}

/// Run the in-process scan loop on a background thread.
fn spawn_internal_miner<L>(
	mining_handle: PowMiningHandle<L>,
	start: U256,
	stride: U256,
	claimed: Arc<Mutex<Option<H256>>>,
)
where
	L: sc_consensus::JustificationSyncLink<Block> + Send + Sync + 'static,
{
	let mut nonce = start;
	let mut best: Option<BestSolution> = None;

	std::thread::spawn(move || loop {
		let metadata = match mining_handle.metadata() {
			Some(m) => m,
			None => {
				std::thread::sleep(Duration::from_millis(100));
				continue;
			},
		};

		let pre_hash = metadata.pre_hash;
		let difficulty = metadata.difficulty;

		// A new head retires the saved work; it was bound to the prior pre_hash.
		if best.as_ref().is_some_and(|b| b.pre_hash != pre_hash) {
			best = None;
		}

		let compute = poscan::Compute { pre_hash, nonce };
		nonce = nonce.overflowing_add(stride).0;

		if let Some(work) = compute.work() {
			let num_hash = U256::from_big_endian(work.as_bytes());
			let max_difficulty = U256::MAX.checked_div(num_hash).unwrap_or(U256::MAX);
			if best.as_ref().is_none_or(|b| max_difficulty > b.max_difficulty) {
				best = Some(BestSolution { pre_hash, nonce: compute.nonce, work, max_difficulty });
			}
		}

		// Difficulty drops over time. Submit once the live target falls within the
		// reach of the strongest work held for this head.
		if let Some(b) = &best
			&& difficulty <= b.max_difficulty
		{
			// One submission per head. A sibling thread may already own this
			// pre_hash, and a second import only drains the runtime instance pool.
			let won = {
				let mut claim = claimed.lock().unwrap();
				let free = *claim != Some(pre_hash);
				if free {
					*claim = Some(pre_hash);
				}
				free
			};

			if won {
				log::debug!(target: "pow", "🎉 Found seal at nonce {} on top of {}, difficulty {}, pre_hash {}", b.nonce, metadata.best_hash, difficulty, pre_hash);
				let seal = poscan::Seal { nonce: b.nonce, work: b.work };
				let encoded_seal = codec::Encode::encode(&seal);
				futures::executor::block_on(mining_handle.submit(pre_hash, encoded_seal));
			}

			best = None;
		}
	});
}

/// Builds a new service for a full client.
pub fn new_full<
	N: sc_network::NetworkBackend<Block, <Block as sp_runtime::traits::Block>::Hash>,
>(
	config: Configuration,
	mining: Option<MiningConfig>,
	eth_rpc: Option<EthRpcConfig>,
) -> Result<TaskManager, ServiceError> {
	let sc_service::PartialComponents {
		client,
		backend,
		mut task_manager,
		import_queue,
		keystore_container,
		select_chain,
		transaction_pool,
		other: (grandpa_block_import, grandpa_link, mut telemetry),
	} = new_partial(&config)?;

	let mut net_config = sc_network::config::FullNetworkConfiguration::<
		Block,
		<Block as sp_runtime::traits::Block>::Hash,
		N,
	>::new(&config.network, config.prometheus_registry().cloned());
	let metrics = N::register_notification_metrics(config.prometheus_registry());

	let peer_store_handle = net_config.peer_store_handle();
	let genesis_hash = client
		.block_hash(0)
		.ok()
		.flatten()
		.expect("Genesis block exists; qed");
	let grandpa_protocol_name = sc_consensus_grandpa::protocol_standard_name(
		&genesis_hash,
		&config.chain_spec,
	);
	let (grandpa_protocol_config, grandpa_notification_service) =
		sc_consensus_grandpa::grandpa_peers_set_config::<_, N>(
			grandpa_protocol_name.clone(),
			metrics.clone(),
			peer_store_handle,
		);
	net_config.add_notification_protocol(grandpa_protocol_config);

	let (network, system_rpc_tx, tx_handler_controller, sync_service, _bitswap_handle) =
		sc_service::build_network(sc_service::BuildNetworkParams {
			config: &config,
			net_config,
			client: client.clone(),
			transaction_pool: transaction_pool.clone(),
			spawn_handle: task_manager.spawn_handle(),
			spawn_essential_handle: task_manager.spawn_essential_handle(),
			import_queue,
			block_announce_validator_builder: None,
			warp_sync_config: None,
			block_relay: None,
			metrics,
			gap_sync_body_policy: None,
		})?;

	if config.offchain_worker.enabled {
		let offchain_workers =
			sc_offchain::OffchainWorkers::new(sc_offchain::OffchainWorkerOptions {
				runtime_api_provider: client.clone(),
				is_validator: config.role.is_authority(),
				keystore: Some(keystore_container.keystore()),
				offchain_db: backend.offchain_storage(),
				transaction_pool: Some(OffchainTransactionPoolFactory::new(
					transaction_pool.clone(),
				)),
				network_provider: Arc::new(network.clone()),
				enable_http_requests: true,
				custom_extensions: |_| vec![],
			})?;
		task_manager.spawn_handle().spawn(
			"offchain-workers-runner",
			"offchain-worker",
			offchain_workers.run(client.clone(), task_manager.spawn_handle()).boxed(),
		);
	}

	let role = config.role;
	let is_authority = role.is_authority();
	let prometheus_registry = config.prometheus_registry().cloned();

	let shared_voter_state = SharedVoterState::empty();
	let shared_authority_set = grandpa_link.shared_authority_set().clone();
	let justification_stream = grandpa_link.justification_stream();
	let finality_proof_provider = sc_consensus_grandpa::FinalityProofProvider::new_for_service(
		backend.clone(),
		Some(shared_authority_set.clone()),
	);

	// ---------- PoW mining (external RPC, plus local scan when node_miner) ----------
	// The proposal worker feeds both paths. Only how a seal is found differs
	// (in-process scan vs external submission).
	let mining_rpc: Option<(Arc<dyn ExternalMiner>, String, String)> =
		if let Some(MiningConfig { miner, node_miner }) = mining {
			let proposer_factory = sc_basic_authorship::ProposerFactory::new(
				task_manager.spawn_handle(),
				client.clone(),
				transaction_pool.clone(),
				prometheus_registry.as_ref(),
				telemetry.as_ref().map(|x| x.handle()),
			);

			let algorithm = PoScanAlgorithm::new(client.clone());

			let pow_block_import = PowBlockImport::new(
				grandpa_block_import.clone(),
				client.clone(),
				algorithm.clone(),
				0u32,
				select_chain.clone(),
				move |_, ()| async { Ok(sp_timestamp::InherentDataProvider::from_system_time()) },
			);

			log::info!(target: "pow", "👛 Miner: {}", miner);
			log::info!(target: "pow", "🧵 Mining thread(s): {}", node_miner);

			let pre_runtime = codec::Encode::encode(&miner);

			let (mining_handle, mining_worker) = sc_consensus_pow::start_mining_worker(
				Box::new(pow_block_import),
				client.clone(),
				select_chain.clone(),
				algorithm,
				proposer_factory,
				sync_service.clone(),
				sync_service.clone(),
				Some(pre_runtime),
				move |_, ()| async { Ok(sp_timestamp::InherentDataProvider::from_system_time()) },
				Duration::from_secs(1),
				Duration::from_secs(2),
			);

			task_manager.spawn_essential_handle().spawn_blocking(
				"pow-mining-worker",
				Some("block-authoring"),
				mining_worker,
			);

			let claimed: Arc<Mutex<Option<H256>>> = Arc::new(Mutex::new(None));
			for i in 0..node_miner {
				spawn_internal_miner(
					mining_handle.clone(),
					U256::from(i),
					U256::from(node_miner),
					claimed.clone(),
				);
			}

			let handle: Arc<dyn ExternalMiner> = Arc::new(mining_handle);
			let protocol = String::from_utf8_lossy(poscan::POSCAN_PROTOCOL).into_owned();
			Some((handle, miner.to_ss58check(), protocol))
		} else {
			None
		};

	let rpc_extensions_builder = {
		let client = client.clone();
		let pool = transaction_pool.clone();
		let shared_voter_state = shared_voter_state.clone();
		let shared_authority_set = shared_authority_set.clone();
		let justification_stream = justification_stream.clone();
		let finality_proof_provider = finality_proof_provider.clone();
		let max_response_size = config.rpc.max_response_size.saturating_mul(MEGABYTE);

		Box::new(move |subscription_executor: sc_rpc::SubscriptionTaskExecutor| {
			let mining = mining_rpc.as_ref().map(|(handle, miner, protocol)| {
				crate::rpc::MiningDeps {
					handle: handle.clone(),
					miner: miner.clone(),
					protocol: protocol.clone(),
				}
			});

			let deps = crate::rpc::FullDeps {
				client: client.clone(),
				pool: pool.clone(),
				grandpa: crate::rpc::GrandpaDeps {
					shared_voter_state: shared_voter_state.clone(),
					shared_authority_set: shared_authority_set.clone(),
					justification_stream: justification_stream.clone(),
					subscription_executor: subscription_executor.clone(),
					finality_provider: finality_proof_provider.clone(),
				},
				mining,
				eth_rpc: eth_rpc.clone(),
				max_response_size,
			};
			crate::rpc::create_full(deps).map_err(Into::into)
		})
	};

	let _rpc_handlers = sc_service::spawn_tasks(sc_service::SpawnTasksParams {
		network: Arc::new(network.clone()),
		client: client.clone(),
		keystore: keystore_container.keystore(),
		task_manager: &mut task_manager,
		transaction_pool: transaction_pool.clone(),
		rpc_builder: rpc_extensions_builder,
		backend: backend.clone(),
		system_rpc_tx,
		tx_handler_controller,
		sync_service: sync_service.clone(),
		config,
		telemetry: telemetry.as_mut(),
		tracing_execute_block: None,
	})?;

	// GRANDPA voter / observer.
	let grandpa_config = sc_consensus_grandpa::Config {
		gossip_duration: Duration::from_secs(2),
		justification_generation_period: GRANDPA_JUSTIFICATION_PERIOD,
		name: None,
		observer_enabled: false,
		keystore: is_authority.then(|| keystore_container.keystore()),
		local_role: role,
		telemetry: telemetry.as_ref().map(|x| x.handle()),
		protocol_name: grandpa_protocol_name,
	};

	let grandpa_params = sc_consensus_grandpa::GrandpaParams {
		config: grandpa_config,
		link: grandpa_link,
		network,
		sync: Arc::new(sync_service.clone()),
		voting_rule: sc_consensus_grandpa::VotingRulesBuilder::default().build(),
		prometheus_registry: prometheus_registry.clone(),
		shared_voter_state,
		telemetry: telemetry.as_ref().map(|x| x.handle()),
		offchain_tx_pool_factory: OffchainTransactionPoolFactory::new(transaction_pool.clone()),
		notification_service: grandpa_notification_service,
	};

	task_manager.spawn_essential_handle().spawn_blocking(
		if is_authority { "grandpa-voter" } else { "grandpa-observer" },
		None,
		sc_consensus_grandpa::run_grandpa_voter(grandpa_params)?,
	);

	Ok(task_manager)
}
