//! What the Frontier history reads off the node.

use std::{cell::RefCell, sync::Arc};

use sc_client_api::{AuxStore, StorageKey, StorageProvider};
use sc_executor::WasmExecutor;
use sp_api::{ApiError, CallApiAt, CallContext};
use sp_blockchain::HeaderBackend;
use sp_core::H256;
use sp_externalities::Extensions;
use sp_runtime::traits::{HashingFor, Header as _};
use sp_state_machine::{
	backend::{AsTrieBackend, BackendRuntimeCode},
	OverlayedChanges, StateMachine, TrieBackendBuilder,
};
use sp_trie::{proof_size_extension::ProofSizeExt, recorder::Recorder};

use numen_runtime::opaque::{Block, Header};

use crate::service::{FullClient, HostFunctions};

/// Storage changes a runtime call runs on top of.
pub type Changes = OverlayedChanges<HashingFor<Block>>;

pub trait Node: Send + Sync + 'static {
	fn best_number(&self) -> u32;

	fn finalized_number(&self) -> u32;

	/// The hash of the block at `number` on the best chain.
	fn hash(&self, number: u32) -> Option<H256>;

	fn header(&self, hash: H256) -> Option<Header>;

	/// The version of a runtime API in the runtime that built block `at`.
	fn api_version(&self, at: H256, api: &[u8; 8]) -> Option<u32>;

	/// Calls `function` of the runtime that built block `at`. The call runs
	/// on that block's state with `changes` on top, which take up its writes.
	/// A metered call gets the storage proof size the way block execution
	/// does.
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

/// The node's client, plus the executor for past runtimes.
pub struct ClientNode {
	pub client: Arc<FullClient>,
	pub executor: WasmExecutor<HostFunctions>,
}

impl ClientNode {
	/// The block whose state holds the runtime that built `at`. That's the
	/// parent, since a block that sets new code already holds it. Genesis is
	/// its own.
	fn built_by(&self, at: H256) -> Result<H256, ApiError> {
		let header = self.header(at).ok_or_else(|| ApiError::UnknownBlock(format!("{at:?}")))?;
		Ok(if header.number == 0 { at } else { *header.parent_hash() })
	}
}

impl Node for ClientNode {
	fn best_number(&self) -> u32 {
		self.client.info().best_number
	}

	fn finalized_number(&self) -> u32 {
		self.client.info().finalized_number
	}

	fn hash(&self, number: u32) -> Option<H256> {
		HeaderBackend::hash(self.client.as_ref(), number).ok().flatten()
	}

	fn header(&self, hash: H256) -> Option<Header> {
		HeaderBackend::header(self.client.as_ref(), hash).ok().flatten()
	}

	fn api_version(&self, at: H256, api: &[u8; 8]) -> Option<u32> {
		let built_by = self.built_by(at).ok()?;
		self.client.runtime_version_at(built_by, CallContext::Offchain).ok()?.api_version(api)
	}

	fn call(
		&self,
		at: H256,
		function: &'static str,
		arguments: Vec<u8>,
		changes: &RefCell<Changes>,
		metered: bool,
	) -> Result<Vec<u8>, ApiError> {
		let state = self.client.state_at(at)?;
		let code_state = self.client.state_at(self.built_by(at)?)?;
		let code = BackendRuntimeCode::new(&code_state, CallContext::Offchain.into());
		let code = code.runtime_code().map_err(sp_blockchain::Error::RuntimeCode)?;
		let changes = &mut *changes.borrow_mut();
		let mut extensions = Extensions::new();
		let context = CallContext::Offchain;
		let result = if metered {
			let recorder = Recorder::default();
			extensions.register(ProofSizeExt::new(recorder.clone()));
			let backend = TrieBackendBuilder::wrap(state.as_trie_backend()).with_recorder(recorder).build();
			StateMachine::new(&backend, changes, &self.executor, function, &arguments, &mut extensions, &code, context)
				.set_parent_hash(at)
				.execute()
		} else {
			StateMachine::new(&state, changes, &self.executor, function, &arguments, &mut extensions, &code, context)
				.set_parent_hash(at)
				.execute()
		};
		Ok(result.map_err(sp_blockchain::Error::from)?)
	}

	fn storage(&self, at: H256, key: &[u8]) -> Option<Vec<u8>> {
		let data = StorageProvider::storage(self.client.as_ref(), at, &StorageKey(key.to_vec()));
		data.ok().flatten().map(|data| data.0)
	}

	fn aux(&self, key: &[u8]) -> Option<Vec<u8>> {
		self.client.get_aux(key).ok().flatten()
	}

	fn insert_aux(&self, entries: &[(Vec<u8>, Vec<u8>)]) -> sp_blockchain::Result<()> {
		let entries: Vec<(&[u8], &[u8])> =
			entries.iter().map(|(key, value)| (key.as_slice(), value.as_slice())).collect();
		AuxStore::insert_aux(self.client.as_ref(), &entries, &[])
	}
}
