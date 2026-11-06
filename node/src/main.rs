//! Numen node CLI library.
#![warn(missing_docs)]
#![allow(clippy::result_large_err)]

mod benchmarking;
mod chain_spec;
mod cli;
mod command;
mod eth_forward;
mod mining_rpc;
mod object_rpc;
mod rpc;
mod service;

fn main() -> sc_cli::Result<()> {
	command::run()
}
