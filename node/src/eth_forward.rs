//! The Ethereum JSON-RPC namespaces on the node's own RPC port.
//!
//! eth-rpc answers for the blocks pallet-revive built. The blocks before them
//! belong to a node that still runs Frontier and stopped importing at the
//! upgrade. Calls about those blocks go there, and so do calls about a block or
//! transaction hash eth-rpc does not know.

use std::{
	error::Error,
	pin::pin,
	sync::{Arc, OnceLock},
	time::Duration,
};

use futures::{future, lock::Mutex};
use jsonrpsee::{
	core::{
		client::{ClientT, Error as ClientError, Subscription, SubscriptionClientT},
		params::ArrayParams,
		RegisterMethodError, SubscriptionResult,
	},
	http_client::{HttpClient, HttpClientBuilder},
	types::{error::INTERNAL_ERROR_CODE, ErrorObjectOwned, Params},
	ws_client::{WsClient, WsClientBuilder},
	PendingSubscriptionSink, RpcModule, SubscriptionMessage, SubscriptionSink,
};
use serde_json::Value;
use sp_blockchain::HeaderBackend;
use sp_runtime::{generic::DigestItem, traits::Header as _};
use tokio::time::sleep;

use numen_runtime::opaque::Block;

/// Engine id of the digest Frontier put in every block it built.
const FRONTIER_ENGINE_ID: [u8; 4] = *b"fron";

/// How often a subscription opened before the switch checks for it.
const SWITCH_POLL: Duration = Duration::from_secs(2);

/// How long the node waits before it reopens a subscription. The wait
/// doubles with each try, up to `RETRY_CAP`, and resets once a notification
/// gets through.
const RETRY_FIRST: Duration = Duration::from_secs(1);
const RETRY_CAP: Duration = Duration::from_secs(8);

/// Where the block a method asks about sits in its parameters.
#[derive(Clone, Copy)]
enum Route {
	/// No block, the method reads the head or sends something.
	Head,
	/// A block number, tag or hash at this position.
	Block(usize),
	/// A block or transaction hash first.
	Hash,
	/// A log filter, which may span both sides.
	Logs,
}

/// The methods eth-rpc serves, each with the block it is about.
const METHODS: &[(&str, Route)] = &[
	("eth_accounts", Route::Head),
	("eth_blockNumber", Route::Head),
	("eth_call", Route::Block(1)),
	("eth_chainId", Route::Head),
	("eth_estimateGas", Route::Block(1)),
	("eth_feeHistory", Route::Head),
	("eth_gasPrice", Route::Head),
	("eth_getBalance", Route::Block(1)),
	("eth_getBlockByHash", Route::Hash),
	("eth_getBlockByNumber", Route::Block(0)),
	("eth_getBlockReceipts", Route::Block(0)),
	("eth_getBlockTransactionCountByHash", Route::Hash),
	("eth_getBlockTransactionCountByNumber", Route::Block(0)),
	("eth_getCode", Route::Block(1)),
	("eth_getLogs", Route::Logs),
	("eth_getStorageAt", Route::Block(2)),
	("eth_getTransactionByBlockHashAndIndex", Route::Hash),
	("eth_getTransactionByBlockNumberAndIndex", Route::Block(0)),
	("eth_getTransactionByHash", Route::Hash),
	("eth_getTransactionCount", Route::Block(1)),
	("eth_getTransactionReceipt", Route::Hash),
	("eth_maxPriorityFeePerGas", Route::Head),
	("eth_sendRawTransaction", Route::Head),
	("eth_sendTransaction", Route::Head),
	("eth_syncing", Route::Head),
	("net_listening", Route::Head),
	("net_peerCount", Route::Head),
	("net_version", Route::Head),
	("web3_clientVersion", Route::Head),
];

/// Which side of the switch holds a block.
#[derive(Debug, PartialEq)]
enum Side {
	Revive,
	Frontier,
	/// Known by hash only, so eth-rpc is asked first.
	Hash,
}

/// What the forwarder reads off the chain.
pub trait Chain: Send + Sync {
	fn best_number(&self) -> u64;

	/// Whether Frontier built the block at `number` on the best chain.
	fn built_by_frontier(&self, number: u64) -> bool;
}

/// The node's client seen as a [`Chain`].
pub struct ClientChain<C>(pub Arc<C>);

impl<C: HeaderBackend<Block> + Send + Sync> Chain for ClientChain<C> {
	fn best_number(&self) -> u64 {
		self.0.info().best_number.into()
	}

	fn built_by_frontier(&self, number: u64) -> bool {
		let Ok(number) = u32::try_from(number) else { return false };
		let Ok(Some(hash)) = self.0.hash(number) else { return false };
		let Ok(Some(header)) = self.0.header(hash) else { return false };
		header.digest().logs().iter().any(
			|item| matches!(item, DigestItem::Consensus(engine, _) if *engine == FRONTIER_ENGINE_ID),
		)
	}
}

/// The first block pallet-revive built, found once the chain has one.
struct Switch {
	chain: Box<dyn Chain>,
	block: OnceLock<u64>,
}

impl Switch {
	fn block(&self) -> Option<u64> {
		if let Some(block) = self.block.get() {
			return Some(*block);
		}
		let best = self.chain.best_number();
		if best == 0 || self.chain.built_by_frontier(best) {
			return None;
		}
		let (mut frontier, mut revive) = (0, best);
		while revive - frontier > 1 {
			let middle = frontier + (revive - frontier) / 2;
			if self.chain.built_by_frontier(middle) {
				frontier = middle;
			} else {
				revive = middle;
			}
		}
		Some(*self.block.get_or_init(|| revive))
	}

	/// Waits until the chain has a block pallet-revive built.
	async fn reached(&self) {
		while self.block().is_none() {
			sleep(SWITCH_POLL).await;
		}
	}
}

/// A JSON-RPC server the node hands calls to.
struct Upstream {
	http: HttpClient,
	ws_url: String,
	max_response_size: u32,
	/// The connection every subscription to this server shares.
	ws: Mutex<Option<Arc<WsClient>>>,
}

impl Upstream {
	fn new(url: &str, max_response_size: u32) -> Result<Self, Box<dyn Error + Send + Sync>> {
		let (http_url, ws_url) = match url.split_once("://") {
			Some(("ws", rest)) | Some(("http", rest)) => (format!("http://{rest}"), format!("ws://{rest}")),
			Some(("wss", rest)) | Some(("https", rest)) =>
				(format!("https://{rest}"), format!("wss://{rest}")),
			_ => return Err(format!("`{url}` is not an http or ws URL").into()),
		};
		let http = HttpClientBuilder::default().max_response_size(max_response_size).build(http_url)?;
		Ok(Self { http, ws_url, max_response_size, ws: Mutex::new(None) })
	}

	async fn call(&self, method: &str, params: &[Value]) -> Result<Value, ClientError> {
		self.http.request(method, array(params)).await
	}

	/// The WebSocket client subscriptions ride on, connected on first use and
	/// again after the connection drops.
	async fn ws(&self) -> Result<Arc<WsClient>, ClientError> {
		let mut shared = self.ws.lock().await;
		if let Some(client) = shared.as_ref().filter(|client| client.is_connected()) {
			return Ok(client.clone());
		}
		let client = WsClientBuilder::default()
			.max_response_size(self.max_response_size)
			.build(&self.ws_url)
			.await?;
		Ok(shared.insert(Arc::new(client)).clone())
	}
}

fn array(params: &[Value]) -> ArrayParams {
	let mut array = ArrayParams::new();
	for param in params {
		array.insert(param).expect("a JSON value serializes");
	}
	array
}

fn upstream_error(error: ClientError) -> ErrorObjectOwned {
	match error {
		ClientError::Call(error) => error,
		other => ErrorObjectOwned::owned(
			INTERNAL_ERROR_CODE,
			format!("Ethereum RPC upstream failed: {other}"),
			None::<()>,
		),
	}
}

/// The subscription the node holds on an upstream for one subscriber.
struct Feed {
	notifications: Subscription<Value>,
	/// Whether the switch had come when the feed was opened.
	switched: bool,
}

impl Feed {
	/// The next notification. None once the upstream ends the subscription, or
	/// once the switch comes for a feed opened before it.
	async fn next(&mut self, switch: &Switch) -> Option<Value> {
		let switched = self.switched;
		let switch_came = async move {
			if switched {
				future::pending().await
			} else {
				switch.reached().await
			}
		};
		match future::select(pin!(self.notifications.next()), pin!(switch_came)).await {
			future::Either::Left((Some(Ok(item)), _)) => Some(item),
			_ => None,
		}
	}
}

/// Hands the Ethereum JSON-RPC methods to eth-rpc, or to the frozen Frontier
/// node for blocks from before pallet-revive.
pub struct EthForward {
	revive: Upstream,
	frontier: Option<Upstream>,
	switch: Switch,
}

impl EthForward {
	/// `eth_rpc` is the eth-rpc server, `frozen` the Frontier node that answers
	/// for the blocks before the switch. `max_response_size` is the node's own
	/// response limit in bytes. A bigger answer could not be served anyway.
	pub fn new(
		chain: impl Chain + 'static,
		eth_rpc: &str,
		frozen: Option<&str>,
		max_response_size: u32,
	) -> Result<Self, Box<dyn Error + Send + Sync>> {
		Ok(Self {
			revive: Upstream::new(eth_rpc, max_response_size)?,
			frontier: frozen.map(|url| Upstream::new(url, max_response_size)).transpose()?,
			switch: Switch { chain: Box::new(chain), block: OnceLock::new() },
		})
	}

	pub fn into_rpc(self) -> Result<RpcModule<()>, RegisterMethodError> {
		let this = Arc::new(self);
		let mut module = RpcModule::new(());
		for &(method, route) in METHODS {
			let this = this.clone();
			module.register_async_method(method, move |params, _, _| {
				let this = this.clone();
				async move { this.forward(method, route, params.parse().unwrap_or_default()).await }
			})?;
		}
		module.register_subscription(
			"eth_subscribe",
			"eth_subscription",
			"eth_unsubscribe",
			move |params, pending, _, _| {
				let this = this.clone();
				async move { this.subscribe(params, pending).await }
			},
		)?;
		Ok(module)
	}

	/// Where calls go before the switch, when Frontier still builds the blocks.
	fn before_switch(&self) -> &Upstream {
		self.frontier.as_ref().unwrap_or(&self.revive)
	}

	async fn forward(
		&self,
		method: &str,
		route: Route,
		params: Vec<Value>,
	) -> Result<Value, ErrorObjectOwned> {
		let Some(switch) = self.switch.block() else {
			return self.before_switch().call(method, &params).await.map_err(upstream_error);
		};
		let side = match route {
			Route::Head => Side::Revive,
			Route::Hash => Side::Hash,
			Route::Block(at) => side(params.get(at), switch),
			Route::Logs => return self.logs(params, switch).await.map_err(upstream_error),
		};
		match (side, &self.frontier) {
			(Side::Frontier, Some(frontier)) => frontier.call(method, &params).await,
			(Side::Hash, Some(frontier)) => match self.revive.call(method, &params).await {
				Ok(Value::Null) | Err(ClientError::Call(_)) => frontier.call(method, &params).await,
				answer => answer,
			},
			_ => self.revive.call(method, &params).await,
		}
		.map_err(upstream_error)
	}

	/// A filter that spans the switch is split there. The Frontier logs come
	/// first, being from earlier blocks.
	async fn logs(&self, params: Vec<Value>, switch: u64) -> Result<Value, ClientError> {
		let (Some(frontier), Some(Value::Object(filter))) = (&self.frontier, params.first()) else {
			return self.revive.call("eth_getLogs", &params).await;
		};
		if filter.contains_key("blockHash") {
			return match self.revive.call("eth_getLogs", &params).await {
				Ok(Value::Array(logs)) if logs.is_empty() => frontier.call("eth_getLogs", &params).await,
				Err(ClientError::Call(_)) => frontier.call("eth_getLogs", &params).await,
				answer => answer,
			};
		}
		let from = number(filter.get("fromBlock"));
		let to = number(filter.get("toBlock"));
		if to < switch {
			return frontier.call("eth_getLogs", &params).await;
		}
		if from >= switch {
			return self.revive.call("eth_getLogs", &params).await;
		}
		let mut early = filter.clone();
		early.insert("toBlock".into(), quantity(switch - 1));
		let mut late = filter.clone();
		late.insert("fromBlock".into(), quantity(switch));
		let (early, late) = future::join(
			frontier.call("eth_getLogs", &[Value::Object(early)]),
			self.revive.call("eth_getLogs", &[Value::Object(late)]),
		)
		.await;
		match (early?, late?) {
			(Value::Array(mut early), Value::Array(late)) => {
				early.extend(late);
				Ok(Value::Array(early))
			},
			(_, late) => Ok(late),
		}
	}

	async fn subscribe(&self, params: Params<'static>, pending: PendingSubscriptionSink) -> SubscriptionResult {
		let params: Vec<Value> = params.parse().unwrap_or_default();
		let feed = match self.open(&params).await {
			Ok(feed) => feed,
			Err(error) => {
				pending.reject(upstream_error(error)).await;
				return Ok(());
			},
		};
		let sink = pending.accept().await?;
		match future::select(pin!(self.relay(&sink, &params, feed)), pin!(sink.closed())).await {
			future::Either::Left((result, _)) => result,
			future::Either::Right(_) => Ok(()),
		}
	}

	/// Opens the subscription on the upstream that takes new ones.
	async fn open(&self, params: &[Value]) -> Result<Feed, ClientError> {
		let switched = self.switch.block().is_some();
		let upstream = if switched { &self.revive } else { self.before_switch() };
		let client = upstream.ws().await?;
		let notifications = client.subscribe("eth_subscribe", array(params), "eth_unsubscribe").await?;
		Ok(Feed { notifications, switched })
	}

	/// Passes every notification on to the subscriber, reopening the feed
	/// whenever it stops.
	async fn relay(&self, sink: &SubscriptionSink, params: &[Value], mut feed: Feed) -> SubscriptionResult {
		let mut wait = RETRY_FIRST;
		loop {
			while let Some(item) = feed.next(&self.switch).await {
				sink.send(SubscriptionMessage::from_json(&item)?).await?;
				wait = RETRY_FIRST;
			}
			feed = self.reopen(params, &mut wait).await;
		}
	}

	/// Reopens the subscription. Waits before every try and keeps trying until
	/// an upstream takes it.
	async fn reopen(&self, params: &[Value], wait: &mut Duration) -> Feed {
		loop {
			sleep(*wait).await;
			*wait = (*wait * 2).min(RETRY_CAP);
			if let Ok(feed) = self.open(params).await {
				return feed;
			}
		}
	}
}

/// The side of the switch `block` lies on, a missing block meaning the head.
fn side(block: Option<&Value>, switch: u64) -> Side {
	match block {
		Some(Value::String(tag)) if tag == "earliest" => Side::Frontier,
		Some(Value::String(hash)) if hash.len() == 66 => Side::Hash,
		Some(Value::String(tag)) => match parse_quantity(tag) {
			Some(number) if number < switch => Side::Frontier,
			_ => Side::Revive,
		},
		Some(Value::Object(block)) if block.contains_key("blockHash") => Side::Hash,
		Some(Value::Object(block)) => side(block.get("blockNumber"), switch),
		_ => Side::Revive,
	}
}

/// The block number a filter bound stands for, the head being the highest.
fn number(bound: Option<&Value>) -> u64 {
	match bound {
		Some(Value::String(tag)) if tag == "earliest" => 0,
		Some(Value::String(tag)) => parse_quantity(tag).unwrap_or(u64::MAX),
		_ => u64::MAX,
	}
}

fn parse_quantity(value: &str) -> Option<u64> {
	u64::from_str_radix(value.strip_prefix("0x")?, 16).ok()
}

fn quantity(number: u64) -> Value {
	Value::String(format!("{number:#x}"))
}

#[cfg(test)]
mod tests {
	use super::*;
	use jsonrpsee::{
		server::{Server, ServerHandle},
		MethodsError,
	};
	use serde_json::json;
	use std::sync::atomic::{AtomicU64, Ordering};

	/// A chain whose blocks from `switch` on carry no Frontier digest.
	struct FakeChain {
		best: u64,
		switch: u64,
	}

	impl Chain for FakeChain {
		fn best_number(&self) -> u64 {
			self.best
		}

		fn built_by_frontier(&self, number: u64) -> bool {
			number < self.switch
		}
	}

	/// A [`FakeChain`] whose head the test moves.
	struct MovingChain {
		best: Arc<AtomicU64>,
		switch: u64,
	}

	impl Chain for MovingChain {
		fn best_number(&self) -> u64 {
			self.best.load(Ordering::Relaxed)
		}

		fn built_by_frontier(&self, number: u64) -> bool {
			number < self.switch
		}
	}

	/// The hash only the frozen node knows.
	const OLD_HASH: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";

	const RESPONSE_LIMIT: u32 = 1 << 20;

	/// An upstream that names itself in every answer and returns one log per
	/// block of a filter's range. Its subscriptions send two heads and stay
	/// open until the subscriber leaves.
	async fn upstream(name: &'static str, max_connections: u32) -> (String, ServerHandle) {
		serve(name, "127.0.0.1:0", max_connections).await
	}

	async fn serve(name: &'static str, addr: &str, max_connections: u32) -> (String, ServerHandle) {
		let mut module = RpcModule::new(());
		for &(method, route) in METHODS {
			module
				.register_method(method, move |params, _, _| {
					let params: Vec<Value> = params.parse().unwrap_or_default();
					if method == "eth_getLogs" {
						let filter = &params[0];
						let from = number(filter.get("fromBlock"));
						let to = number(filter.get("toBlock")).min(20);
						return Ok::<_, ErrorObjectOwned>(json!((from..=to)
							.map(|block| json!({ "blockNumber": quantity(block), "from": name }))
							.collect::<Vec<_>>()));
					}
					if name == "eth-rpc" && serde_json::to_string(&params).unwrap().contains(OLD_HASH) {
						return match route {
							Route::Hash => Ok(Value::Null),
							_ => Err(ErrorObjectOwned::owned(-32000, "block not found", None::<()>)),
						};
					}
					if method == "eth_chainId" {
						return Err(ErrorObjectOwned::owned(-32001, "rate limited", Some("try later")));
					}
					Ok(json!({ "from": name, "params": params }))
				})
				.unwrap();
		}
		module
			.register_subscription(
				"eth_subscribe",
				"eth_subscription",
				"eth_unsubscribe",
				move |_, pending, _, _| async move {
					let sink = pending.accept().await?;
					for head in 1..=2 {
						sink.send(SubscriptionMessage::from_json(&json!({ "number": head, "from": name }))?)
							.await?;
					}
					sink.closed().await;
					Ok(())
				},
			)
			.unwrap();
		let server = Server::builder().max_connections(max_connections).build(addr).await.unwrap();
		let url = format!("http://{}", server.local_addr().unwrap());
		(url, server.start(module))
	}

	struct Setup {
		module: RpcModule<()>,
		_servers: Vec<ServerHandle>,
	}

	async fn setup(chain: impl Chain + 'static, with_frozen: bool) -> Setup {
		let (eth_rpc, eth_rpc_server) = upstream("eth-rpc", 100).await;
		let (frozen, frozen_server) = upstream("frozen", 100).await;
		let forward =
			EthForward::new(chain, &eth_rpc, with_frozen.then_some(frozen.as_str()), RESPONSE_LIMIT).unwrap();
		Setup { module: forward.into_rpc().unwrap(), _servers: vec![eth_rpc_server, frozen_server] }
	}

	/// The chain switched to pallet-revive at block 10.
	fn switched() -> FakeChain {
		FakeChain { best: 15, switch: 10 }
	}

	/// Awaits `future`, failing the test after 30 seconds instead of hanging.
	async fn soon<T>(future: impl Future<Output = T>) -> T {
		tokio::time::timeout(Duration::from_secs(30), future).await.expect("timed out")
	}

	async fn answered_by(setup: &Setup, method: &str, params: Value) -> String {
		let answer: Value = setup.module.call(method, array(params.as_array().unwrap())).await.unwrap();
		answer["from"].as_str().unwrap().to_owned()
	}

	#[tokio::test]
	async fn blocks_before_the_switch_go_to_the_frozen_node() {
		let setup = setup(switched(), true).await;
		let address = "0x00000000000000000000000000000000000000aa";

		assert_eq!(answered_by(&setup, "eth_getBalance", json!([address, "0x9"])).await, "frozen");
		assert_eq!(answered_by(&setup, "eth_getBalance", json!([address, "earliest"])).await, "frozen");
		assert_eq!(
			answered_by(&setup, "eth_getStorageAt", json!([address, "0x0", { "blockNumber": "0x3" }])).await,
			"frozen",
		);
		assert_eq!(answered_by(&setup, "eth_getBlockByNumber", json!(["0x1", false])).await, "frozen");
	}

	#[tokio::test]
	async fn the_switch_block_and_the_head_go_to_eth_rpc() {
		let setup = setup(switched(), true).await;
		let address = "0x00000000000000000000000000000000000000aa";

		for block in ["0xa", "0xf", "latest", "pending", "safe", "finalized"] {
			assert_eq!(answered_by(&setup, "eth_getBalance", json!([address, block])).await, "eth-rpc", "{block}");
		}
		assert_eq!(answered_by(&setup, "eth_call", json!([{ "to": address }])).await, "eth-rpc");
		assert_eq!(answered_by(&setup, "eth_sendRawTransaction", json!(["0x02"])).await, "eth-rpc");
	}

	#[tokio::test]
	async fn a_hash_eth_rpc_does_not_know_goes_to_the_frozen_node() {
		let setup = setup(switched(), true).await;
		let new_hash = format!("0x{}", "22".repeat(32));

		assert_eq!(answered_by(&setup, "eth_getTransactionReceipt", json!([OLD_HASH])).await, "frozen");
		assert_eq!(answered_by(&setup, "eth_getTransactionReceipt", json!([new_hash])).await, "eth-rpc");
		assert_eq!(
			answered_by(&setup, "eth_getBalance", json!(["0x00", { "blockHash": OLD_HASH }])).await,
			"frozen",
		);
	}

	/// The frozen node's logs end right before the switch and eth-rpc's start
	/// at it, so the merged answer runs through in block order.
	#[tokio::test]
	async fn logs_across_the_switch_come_back_in_block_order() {
		let setup = setup(switched(), true).await;

		let logs: Vec<Value> = setup
			.module
			.call("eth_getLogs", array(&[json!({ "fromBlock": "0x8", "toBlock": "0xb" })]))
			.await
			.unwrap();

		let blocks: Vec<(&str, &str)> = logs
			.iter()
			.map(|log| (log["blockNumber"].as_str().unwrap(), log["from"].as_str().unwrap()))
			.collect();
		assert_eq!(
			blocks,
			[("0x8", "frozen"), ("0x9", "frozen"), ("0xa", "eth-rpc"), ("0xb", "eth-rpc")],
		);
	}

	#[tokio::test]
	async fn logs_on_one_side_of_the_switch_stay_there() {
		let setup = setup(switched(), true).await;

		let early: Vec<Value> = setup
			.module
			.call("eth_getLogs", array(&[json!({ "fromBlock": "0x1", "toBlock": "0x2" })]))
			.await
			.unwrap();
		let late: Vec<Value> = setup
			.module
			.call("eth_getLogs", array(&[json!({ "fromBlock": "0xc", "toBlock": "latest" })]))
			.await
			.unwrap();

		assert!(early.iter().all(|log| log["from"] == "frozen"));
		assert!(late.iter().all(|log| log["from"] == "eth-rpc"));
	}

	#[tokio::test]
	async fn without_a_frozen_node_eth_rpc_answers_everything() {
		let setup = setup(switched(), false).await;

		assert_eq!(answered_by(&setup, "eth_getBlockByNumber", json!(["0x1", false])).await, "eth-rpc");
		let receipt: Value =
			setup.module.call("eth_getTransactionReceipt", array(&[json!(OLD_HASH)])).await.unwrap();
		assert_eq!(receipt, Value::Null);
	}

	#[tokio::test]
	async fn before_the_switch_the_frozen_node_answers_everything() {
		let setup = setup(FakeChain { best: 15, switch: 100 }, true).await;

		assert_eq!(answered_by(&setup, "eth_getBalance", json!(["0x00", "latest"])).await, "frozen");
		assert_eq!(answered_by(&setup, "eth_sendRawTransaction", json!(["0x02"])).await, "frozen");
	}

	#[tokio::test]
	async fn upstream_errors_reach_the_caller_as_they_are() {
		let setup = setup(switched(), true).await;

		let error = setup.module.call::<_, Value>("eth_chainId", ArrayParams::new()).await.unwrap_err();

		let MethodsError::JsonRpc(error) = error else { panic!("a call error, got {error:?}") };
		assert_eq!((error.code(), error.message()), (-32001, "rate limited"));
		assert_eq!(error.data().map(|data| data.get()), Some("\"try later\""));
	}

	#[tokio::test]
	async fn subscriptions_relay_what_eth_rpc_sends() {
		let setup = setup(switched(), true).await;

		let mut heads = setup
			.module
			.subscribe_unbounded("eth_subscribe", array(&[json!("newHeads")]))
			.await
			.unwrap();
		let mut received = Vec::new();
		while received.len() < 2 {
			let (head, _) = heads.next::<Value>().await.unwrap().unwrap();
			received.push(head);
		}

		assert_eq!(received, [json!({ "number": 1, "from": "eth-rpc" }), json!({ "number": 2, "from": "eth-rpc" })]);
	}

	/// The upstream takes one connection. Two subscriptions held open at once
	/// both get served, so they ride the same one.
	#[tokio::test]
	async fn subscriptions_share_one_connection_to_the_upstream() {
		let (eth_rpc, _server) = upstream("eth-rpc", 1).await;
		let module = EthForward::new(switched(), &eth_rpc, None, RESPONSE_LIMIT).unwrap().into_rpc().unwrap();

		let mut first = module.subscribe_unbounded("eth_subscribe", array(&[json!("newHeads")])).await.unwrap();
		let mut second = module.subscribe_unbounded("eth_subscribe", array(&[json!("newHeads")])).await.unwrap();

		for heads in [&mut first, &mut second] {
			let (head, _) = heads.next::<Value>().await.unwrap().unwrap();
			assert_eq!(head, json!({ "number": 1, "from": "eth-rpc" }));
		}
	}

	/// The upstream stays down past the first retry, then comes back on the
	/// same address. The open subscription carries on over a fresh connection.
	#[tokio::test]
	async fn a_subscription_outlives_an_upstream_restart() {
		let (eth_rpc, server) = upstream("eth-rpc", 1).await;
		let module = EthForward::new(switched(), &eth_rpc, None, RESPONSE_LIMIT).unwrap().into_rpc().unwrap();
		let mut heads = module.subscribe_unbounded("eth_subscribe", array(&[json!("newHeads")])).await.unwrap();
		for _ in 0..2 {
			heads.next::<Value>().await.unwrap().unwrap();
		}

		server.stop().unwrap();
		server.stopped().await;
		sleep(RETRY_FIRST * 2).await;
		let _server = serve("eth-rpc", eth_rpc.strip_prefix("http://").unwrap(), 1).await;

		let (head, _) = soon(heads.next::<Value>()).await.unwrap().unwrap();
		assert_eq!(head, json!({ "number": 1, "from": "eth-rpc" }));
	}

	/// The head moves from the last Frontier block onto the switch while the
	/// subscription sits on the frozen node.
	#[tokio::test]
	async fn a_subscription_opened_before_the_switch_moves_to_eth_rpc() {
		let switch = 10;
		let best = Arc::new(AtomicU64::new(switch - 1));
		let setup = setup(MovingChain { best: best.clone(), switch }, true).await;
		let mut heads = setup
			.module
			.subscribe_unbounded("eth_subscribe", array(&[json!("newHeads")]))
			.await
			.unwrap();
		for number in 1..=2 {
			let (head, _) = heads.next::<Value>().await.unwrap().unwrap();
			assert_eq!(head, json!({ "number": number, "from": "frozen" }));
		}

		best.store(switch, Ordering::Relaxed);

		let (head, _) = soon(heads.next::<Value>()).await.unwrap().unwrap();
		assert_eq!(head, json!({ "number": 1, "from": "eth-rpc" }));
	}

	#[test]
	fn the_switch_is_the_first_block_without_a_frontier_digest() {
		for switch in [1, 2, 7, 15] {
			let found = Switch { chain: Box::new(FakeChain { best: 15, switch }), block: OnceLock::new() };
			assert_eq!(found.block(), Some(switch));
		}
		let pending = Switch { chain: Box::new(FakeChain { best: 15, switch: 16 }), block: OnceLock::new() };
		assert_eq!(pending.block(), None);
	}
}
