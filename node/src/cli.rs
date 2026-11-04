use sc_cli::RunCmd;

#[derive(Debug, clap::Parser)]
pub struct Cli {
	#[command(subcommand)]
	pub subcommand: Option<Subcommand>,

	#[clap(flatten)]
	pub run: RunCmd,

	/// Enable PoW mining for the given SS58 reward address and expose the mining
	/// RPC for external miners.
	#[arg(long, value_name = "SS58")]
	pub miner: Option<String>,

	/// Also run the in-process scan loop so the node mines locally. Bare flag uses
	/// all cores. Pass a number to cap the scan threads.
	#[arg(long, value_name = "THREADS", num_args = 0..=1, default_missing_value = "0")]
	pub node_miner: Option<usize>,

	/// Serve the Ethereum JSON-RPC methods by handing them to this eth-rpc
	/// server.
	#[arg(long, value_name = "URL")]
	pub eth_rpc: Option<String>,

	/// Node that still runs Frontier and answers for the blocks before
	/// pallet-revive.
	#[arg(long, value_name = "URL", requires = "eth_rpc")]
	pub frozen_eth_rpc: Option<String>,
}

#[derive(Debug, clap::Subcommand)]
#[allow(clippy::large_enum_variant)]
pub enum Subcommand {
	/// Key management cli utilities
	#[command(subcommand)]
	Key(sc_cli::KeySubcommand),

	/// Build a chain specification.
	BuildSpec(sc_cli::BuildSpecCmd),

	/// Validate blocks.
	CheckBlock(sc_cli::CheckBlockCmd),

	/// Export blocks.
	ExportBlocks(sc_cli::ExportBlocksCmd),

	/// Export the state of a given block into a chain spec.
	ExportState(sc_cli::ExportStateCmd),

	/// Import blocks.
	ImportBlocks(sc_cli::ImportBlocksCmd),

	/// Remove the whole chain.
	PurgeChain(sc_cli::PurgeChainCmd),

	/// Revert the chain to a previous state.
	Revert(sc_cli::RevertCmd),

	/// Sub-commands concerned with benchmarking.
	#[command(subcommand)]
	Benchmark(frame_benchmarking_cli::BenchmarkCmd),

	/// Db meta columns information.
	ChainInfo(sc_cli::ChainInfoCmd),
}
