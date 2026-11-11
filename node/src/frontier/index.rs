//! Finds a Frontier block or transaction by its Ethereum hash.
//!
//! Every block Frontier built carries the hashes of its Ethereum block and
//! transactions in a header digest. The finalized ones are indexed in the
//! node's auxiliary storage. The blocks above them are read off their
//! headers.

use std::sync::{Arc, Mutex, PoisonError};

use codec::{Decode, Encode};
use sp_core::H256;
use sp_runtime::{generic::DigestItem, traits::Header as _};

use super::{node::Node, runtime};

/// Engine id of the digest Frontier put in every block it built.
pub const FRONTIER_ENGINE_ID: [u8; 4] = *b"fron";

/// Holds the number of the first block not indexed yet.
const NEXT: &[u8] = b"frontier:next";

/// Blocks written to the auxiliary storage at a time.
const BATCH: u32 = 4096;

/// Keys and values of the auxiliary storage.
type Entries = Vec<(Vec<u8>, Vec<u8>)>;

/// The Ethereum hashes a block carries in its header.
#[derive(Decode)]
pub struct Hashes {
	pub block_hash: H256,
	pub transaction_hashes: Vec<H256>,
}

#[derive(Decode)]
enum PostLog {
	#[codec(index = 1)]
	Hashes(Hashes),
}

fn block_key(hash: &H256) -> Vec<u8> {
	[b"frontier:block:", hash.as_bytes()].concat()
}

fn transaction_key(hash: &H256) -> Vec<u8> {
	[b"frontier:transaction:", hash.as_bytes()].concat()
}

pub struct Index<N> {
	node: Arc<N>,
	writing: Mutex<()>,
}

impl<N: Node> Index<N> {
	pub fn new(node: Arc<N>) -> Self {
		Self { node, writing: Mutex::new(()) }
	}

	/// The Ethereum hashes of the block at `number`. None when Frontier did
	/// not build that block.
	pub fn hashes(&self, number: u32) -> Option<Hashes> {
		let hash = self.node.hash(number)?;
		if number == 0 {
			let genesis = runtime::block_data(self.node.as_ref(), hash)?;
			return Some(Hashes { block_hash: genesis.block.header.hash(), transaction_hashes: Vec::new() });
		}
		self.node.header(hash)?.digest().logs().iter().find_map(|item| match item {
			DigestItem::Consensus(engine, data) if *engine == FRONTIER_ENGINE_ID =>
				PostLog::decode(&mut &data[..]).ok().map(|PostLog::Hashes(hashes)| hashes),
			_ => None,
		})
	}

	/// The number of the block with this Ethereum hash.
	pub fn block(&self, hash: H256) -> Option<u32> {
		self.catch_up();
		self.read(&block_key(&hash))
			.or_else(|| self.unindexed().find(|(_, hashes)| hashes.block_hash == hash).map(|(number, _)| number))
	}

	/// The block number and position of the transaction with this hash.
	pub fn transaction(&self, hash: H256) -> Option<(u32, u32)> {
		self.catch_up();
		self.read(&transaction_key(&hash)).or_else(|| {
			self.unindexed().find_map(|(number, hashes)| {
				let position = hashes.transaction_hashes.iter().position(|known| *known == hash)?;
				Some((number, u32::try_from(position).ok()?))
			})
		})
	}

	/// Indexes the finalized Frontier blocks not indexed yet.
	pub fn catch_up(&self) {
		let _writing = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
		let finalized = self.node.finalized_number();
		let mut next = self.next();
		while next <= finalized {
			let end = finalized.min(next.saturating_add(BATCH - 1));
			let (entries, indexed) = self.entries(next, end);
			if indexed == next || self.node.insert_aux(&entries).is_err() {
				return;
			}
			next = indexed;
		}
	}

	/// The index entries of the blocks from `start` to `end`, with the block
	/// they stop before. They stop early at a block Frontier did not build.
	pub(super) fn entries(&self, start: u32, end: u32) -> (Entries, u32) {
		let mut entries = Vec::new();
		let mut next = start;
		while next <= end {
			let Some(hashes) = self.hashes(next) else { break };
			entries.push((block_key(&hashes.block_hash), next.encode()));
			for (position, hash) in (0u32..).zip(&hashes.transaction_hashes) {
				entries.push((transaction_key(hash), (next, position).encode()));
			}
			next += 1;
		}
		entries.push((NEXT.to_vec(), next.encode()));
		(entries, next)
	}

	fn next(&self) -> u32 {
		self.read(NEXT).unwrap_or(0)
	}

	fn read<T: Decode>(&self, key: &[u8]) -> Option<T> {
		T::decode(&mut &self.node.aux(key)?[..]).ok()
	}

	/// The Frontier blocks from the first one not indexed up to the best one.
	fn unindexed(&self) -> impl Iterator<Item = (u32, Hashes)> + '_ {
		(self.next()..=self.node.best_number()).map_while(|number| Some((number, self.hashes(number)?)))
	}
}

#[cfg(test)]
mod tests {
	use std::sync::atomic::Ordering;

	use sp_core::U256;

	use super::*;
	use crate::frontier::fake::{outcome, transaction, FakeNode};

	fn ethereum_hash(number: u32) -> H256 {
		H256::from_low_u64_be(1_000_000 + u64::from(number))
	}

	/// A chain of empty Frontier blocks up to `best`, finalized up to
	/// `finalized`.
	fn chain(best: u32, finalized: u32) -> FakeNode {
		let mut node = FakeNode::new();
		for number in 1..=best {
			node.frontier_block(number, ethereum_hash(number), Vec::new());
		}
		node.finalize(finalized);
		node
	}

	fn index(node: FakeNode) -> (Arc<FakeNode>, Index<FakeNode>) {
		let node = Arc::new(node);
		(node.clone(), Index::new(node))
	}

	/// Blocks 0 to 3 are finalized, and the index holds one entry for each
	/// plus the mark of where it stopped.
	#[test]
	fn finalized_blocks_are_indexed_and_the_ones_above_read_off_their_headers() {
		let (node, index) = index(chain(5, 3));

		assert_eq!(index.block(ethereum_hash(3)), Some(3));
		assert_eq!(index.block(ethereum_hash(4)), Some(4));
		assert_eq!(node.aux_entries(), 4 + 1);
	}

	#[test]
	fn a_hash_no_block_carries_is_not_found() {
		let (_, index) = index(chain(5, 3));

		assert_eq!(index.block(ethereum_hash(6)), None);
		assert_eq!(index.transaction(ethereum_hash(3)), None);
	}

	#[test]
	fn indexing_stops_at_the_first_block_frontier_did_not_build() {
		let mut node = chain(3, 0);
		node.revive_block(4);
		node.revive_block(5);
		node.finalize(5);
		let (node, index) = index(node);

		index.catch_up();
		let writes = node.aux_writes.load(Ordering::Relaxed);
		index.catch_up();

		assert_eq!(node.aux_entries(), 4 + 1);
		assert_eq!(node.aux_writes.load(Ordering::Relaxed), writes);
		assert_eq!(index.block(ethereum_hash(3)), Some(3));
	}

	#[test]
	fn indexing_goes_on_past_one_batch() {
		let (node, index) = index(chain(BATCH + 1, BATCH + 1));

		index.catch_up();

		assert_eq!(node.aux_entries() as u32, BATCH + 2 + 1);
		for number in [BATCH - 1, BATCH, BATCH + 1] {
			assert_eq!(index.block(ethereum_hash(number)), Some(number), "{number}");
		}
	}

	#[test]
	fn blocks_finalized_later_are_indexed_at_the_next_lookup() {
		let (node, index) = index(chain(4, 2));
		index.catch_up();
		let before = node.aux_entries();

		node.finalize(4);
		index.block(ethereum_hash(4));

		assert_eq!(node.aux_entries(), before + 2);
	}

	/// Block 1 is finalized and holds two transactions. Block 2 is not and
	/// holds one.
	#[test]
	fn a_transaction_is_found_with_its_block_and_position() {
		let mut node = FakeNode::new();
		let sent = |nonce| (transaction(nonce, 0, 1), outcome(21_000, Vec::new()));
		let finalized = node.hold(1, U256::one(), vec![sent(0), sent(1)]);
		let above = node.hold(2, U256::one(), vec![sent(2)]);
		node.finalize(1);
		let (_, index) = index(node);

		assert_eq!(index.transaction(finalized.transactions[0].hash()), Some((1, 0)));
		assert_eq!(index.transaction(finalized.transactions[1].hash()), Some((1, 1)));
		assert_eq!(index.transaction(above.transactions[0].hash()), Some((2, 0)));
	}
}
