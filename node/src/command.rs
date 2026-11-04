use crate::{
	benchmarking::{inherent_benchmark_data, RemarkBuilder, TransferKeepAliveBuilder},
	chain_spec,
	cli::{Cli, Subcommand},
	service,
};
use frame_benchmarking_cli::{BenchmarkCmd, ExtrinsicFactory, SUBSTRATE_REFERENCE_HARDWARE};
use sc_cli::SubstrateCli;
use sc_service::PartialComponents;
use numen_runtime::{Block, EXISTENTIAL_DEPOSIT};
use sp_core::crypto::Ss58Codec;
use sp_keyring::Sr25519Keyring;

impl SubstrateCli for Cli {
	fn impl_name() -> String {
		"Numen Node".into()
	}

	fn impl_version() -> String {
		env!("SUBSTRATE_CLI_IMPL_VERSION").into()
	}

	fn description() -> String {
		env!("CARGO_PKG_DESCRIPTION").into()
	}

	fn author() -> String {
		env!("CARGO_PKG_AUTHORS").into()
	}

	fn support_url() -> String {
		"https://github.com/numen-network/numen/issues".into()
	}

	fn copyright_start_year() -> i32 {
		2026
	}

	fn load_spec(&self, id: &str) -> Result<Box<dyn sc_service::ChainSpec>, String> {
		match id {
			"dev" => Ok(Box::new(chain_spec::development_chain_spec()?)),
			"integration" => Ok(Box::new(chain_spec::integration_chain_spec()?)),
			"local" => Ok(Box::new(chain_spec::local_chain_spec()?)),
			"testnet" => Ok(Box::new(chain_spec::testnet_chain_spec()?)),
			"mainnet" => Ok(Box::new(chain_spec::mainnet_chain_spec()?)),
			"" => Err(
				"Chain specification not provided. Please specify it with the argument `--chain <SPEC>`."
					.to_owned(),
			),
			path => Ok(Box::new(chain_spec::ChainSpec::from_json_file(std::path::PathBuf::from(
				path,
			))?)),
		}
	}
}

/// Parse and run command line arguments
pub fn run() -> sc_cli::Result<()> {

	sp_core::crypto::set_default_ss58_version(
		numen_runtime::configs::SS58Prefix::get().into(),
	);

	let cli = Cli::from_args();

	match &cli.subcommand {
		Some(Subcommand::Key(cmd)) => cmd.run(&cli),
		Some(Subcommand::BuildSpec(cmd)) => {
			let runner = cli.create_runner(cmd)?;
			runner.sync_run(|config| cmd.run(config.chain_spec, config.network))
		},
		Some(Subcommand::CheckBlock(cmd)) => {
			let runner = cli.create_runner(cmd)?;
			runner.async_run(|config| {
				let PartialComponents { client, task_manager, import_queue, .. } =
					service::new_partial(&config)?;
				Ok((cmd.run(client, import_queue), task_manager))
			})
		},
		Some(Subcommand::ExportBlocks(cmd)) => {
			let runner = cli.create_runner(cmd)?;
			runner.async_run(|config| {
				let PartialComponents { client, task_manager, .. } = service::new_partial(&config)?;
				Ok((cmd.run(client, config.database), task_manager))
			})
		},
		Some(Subcommand::ExportState(cmd)) => {
			let runner = cli.create_runner(cmd)?;
			runner.async_run(|config| {
				let PartialComponents { client, task_manager, .. } = service::new_partial(&config)?;
				Ok((cmd.run(client, config.chain_spec), task_manager))
			})
		},
		Some(Subcommand::ImportBlocks(cmd)) => {
			let runner = cli.create_runner(cmd)?;
			runner.async_run(|config| {
				let PartialComponents { client, task_manager, import_queue, .. } =
					service::new_partial(&config)?;
				Ok((cmd.run(client, import_queue), task_manager))
			})
		},
		Some(Subcommand::PurgeChain(cmd)) => {
			let runner = cli.create_runner(cmd)?;
			runner.sync_run(|config| cmd.run(config.database))
		},
		Some(Subcommand::Revert(cmd)) => {
			let runner = cli.create_runner(cmd)?;
			runner.async_run(|config| {
				let PartialComponents { client, task_manager, backend, .. } =
					service::new_partial(&config)?;
				Ok((cmd.run(client, backend, None), task_manager))
			})
		},
		Some(Subcommand::Benchmark(cmd)) => {
			let runner = cli.create_runner(cmd)?;

			runner.sync_run(|config| {
				// This switch needs to be in the client, since the client decides
				// which sub-commands it wants to support.
				match cmd {
					BenchmarkCmd::Pallet(cmd) => {
						if !cfg!(feature = "runtime-benchmarks") {
							return Err(
								"Runtime benchmarking wasn't enabled when building the node. \
							You can enable it with `--features runtime-benchmarks`."
									.into(),
							);
						}

						cmd.run_with_spec::<
							sp_runtime::traits::HashingFor<Block>,
							cumulus_primitives_proof_size_hostfunction::storage_proof_size::HostFunctions,
						>(Some(config.chain_spec))
					},
					BenchmarkCmd::Block(cmd) => {
						let PartialComponents { client, .. } = service::new_partial(&config)?;
						cmd.run(client)
					},
					#[cfg(not(feature = "runtime-benchmarks"))]
					BenchmarkCmd::Storage(_) => Err(
						"Storage benchmarking can be enabled with `--features runtime-benchmarks`."
							.into(),
					),
					#[cfg(feature = "runtime-benchmarks")]
					BenchmarkCmd::Storage(cmd) => {
						let PartialComponents { client, backend, .. } =
							service::new_partial(&config)?;
						let db = backend.expose_db();
						let storage = backend.expose_storage();
					
						cmd.run(config, client, db, storage, None)
					},
					BenchmarkCmd::Overhead(cmd) => {
						let PartialComponents { client, .. } = service::new_partial(&config)?;
						let ext_builder = RemarkBuilder::new(client.clone());

						cmd.run(
							config.chain_spec.name().into(),
							client,
							inherent_benchmark_data()?,
							Vec::new(),
							&ext_builder,
							false,
						)
					},
					BenchmarkCmd::Extrinsic(cmd) => {
						let PartialComponents { client, .. } = service::new_partial(&config)?;
						// Register the *Remark* and *TKA* builders.
						let ext_factory = ExtrinsicFactory(vec![
							Box::new(RemarkBuilder::new(client.clone())),
							Box::new(TransferKeepAliveBuilder::new(
								client.clone(),
								Sr25519Keyring::Alice.to_account_id(),
								EXISTENTIAL_DEPOSIT,
							)),
						]);

						cmd.run(client, inherent_benchmark_data()?, Vec::new(), &ext_factory)
					},
					BenchmarkCmd::Machine(cmd) =>
						cmd.run(&config, SUBSTRATE_REFERENCE_HARDWARE.clone()),
				}
			})
		},
		Some(Subcommand::ChainInfo(cmd)) => {
			let runner = cli.create_runner(cmd)?;
			runner.sync_run(|config| cmd.run::<Block>(&config))
		},
		None => {
			let miner_account = cli
				.miner
				.as_deref()
				.map(|addr| {
					numen_runtime::AccountId::from_ss58check(addr)
						.map_err(|e| format!("invalid --miner address `{}`: {:?}", addr, e))
				})
				.transpose()
				.map_err(sc_cli::Error::Input)?;

			if miner_account.is_none() && cli.node_miner.is_some() {
				return Err(sc_cli::Error::Input("--node-miner requires --miner".into()));
			}

			let node_miner = match cli.node_miner {
				None => 0,
				Some(0) => std::thread::available_parallelism().map(|c| c.get()).unwrap_or(1),
				Some(n) => n,
			};

			let mining = miner_account.map(|miner| service::MiningConfig { miner, node_miner });
			let eth_rpc = cli
				.eth_rpc
				.clone()
				.map(|eth_rpc| service::EthRpcConfig { eth_rpc, frozen: cli.frozen_eth_rpc.clone() });

			let runner = cli.create_runner(&cli.run)?;
			runner.run_node_until_exit(|config| async move {
				match config.network.network_backend {
					sc_network::config::NetworkBackendType::Libp2p => service::new_full::<
						sc_network::NetworkWorker<
							numen_runtime::opaque::Block,
							<numen_runtime::opaque::Block as sp_runtime::traits::Block>::Hash,
						>,
					>(config, mining, eth_rpc)
					.map_err(sc_cli::Error::Service),
					sc_network::config::NetworkBackendType::Litep2p =>
						service::new_full::<sc_network::Litep2pNetworkBackend>(config, mining, eth_rpc)
							.map_err(sc_cli::Error::Service),
				}
			})
		},
	}
}
