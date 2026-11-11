//! What the Frontier history reads off the node.

use std::cell::RefCell;

use sc_client_api::{AuxStore, StorageKey, StorageProvider};
use sp_api::{ApiError, CallApiAt, CallApiAtParams, CallContext};
use sp_blockchain::HeaderBackend;
use sp_core::H256;
use sp_externalities::Extensions;
use sp_runtime::traits::HashingFor;
use sp_state_machine::OverlayedChanges;
use sp_trie::{proof_size_extension::ProofSizeExt, recorder::Recorder};

use numen_runtime::opaque::{Block, Header};

use crate::service::FullClient;

/// Storage changes a runtime call runs on top of.
pub type Changes = OverlayedChanges<HashingFor<Block>>;

pub trait Node: Send + Sync + 'static {
	fn best_number(&self) -> u32;

	fn finalized_number(&self) -> u32;

	/// The hash of the block at `number` on the best chain.
	fn hash(&self, number: u32) -> Option<H256>;

	fn header(&self, hash: H256) -> Option<Header>;

	/// The version of a runtime API in the runtime of block `at`.
	fn api_version(&self, at: H256, api: &[u8; 8]) -> Option<u32>;

	/// Calls `function` of the runtime of block `at` on top of `changes`,
	/// which take up what the call writes. A metered call gets the storage
	/// proof size the way block execution does.
	fn call(
		&self,
		at: H256,
		function: &'static str,
		arguments: Vec<u8>,
		changes: &RefCell<Changes>,
		metered: bool,
	) -> Result<Vec<u8>, ApiError>;

	fn storage(&self, at: H256, key: &[u8]) -> Option<Vec<u8>>;

	fn aux(&self, key: &[u8]) -> Option<Vec<u8>>;

	fn insert_aux(&self, entries: &[(Vec<u8>, Vec<u8>)]) -> sp_blockchain::Result<()>;
}

impl Node for FullClient {
	fn best_number(&self) -> u32 {
		self.info().best_number
	}

	fn finalized_number(&self) -> u32 {
		self.info().finalized_number
	}

	fn hash(&self, number: u32) -> Option<H256> {
		HeaderBackend::hash(self, number).ok().flatten()
	}

	fn header(&self, hash: H256) -> Option<Header> {
		HeaderBackend::header(self, hash).ok().flatten()
	}

	fn api_version(&self, at: H256, api: &[u8; 8]) -> Option<u32> {
		self.runtime_version_at(at, CallContext::Offchain).ok()?.api_version(api)
	}

	fn call(
		&self,
		at: H256,
		function: &'static str,
		arguments: Vec<u8>,
		changes: &RefCell<Changes>,
		metered: bool,
	) -> Result<Vec<u8>, ApiError> {
		let recorder = metered.then(Recorder::default);
		let mut extensions = Extensions::new();
		if let Some(recorder) = &recorder {
			extensions.register(ProofSizeExt::new(recorder.clone()));
		}
		self.call_api_at(CallApiAtParams {
			at,
			function,
			arguments,
			overlayed_changes: changes,
			call_context: CallContext::Offchain,
			recorder: &recorder,
			extensions: &RefCell::new(extensions),
		})
	}

	fn storage(&self, at: H256, key: &[u8]) -> Option<Vec<u8>> {
		StorageProvider::storage(self, at, &StorageKey(key.to_vec())).ok().flatten().map(|data| data.0)
	}

	fn aux(&self, key: &[u8]) -> Option<Vec<u8>> {
		self.get_aux(key).ok().flatten()
	}

	fn insert_aux(&self, entries: &[(Vec<u8>, Vec<u8>)]) -> sp_blockchain::Result<()> {
		let entries: Vec<(&[u8], &[u8])> =
			entries.iter().map(|(key, value)| (key.as_slice(), value.as_slice())).collect();
		AuxStore::insert_aux(self, &entries, &[])
	}
}
