frame_benchmarking::define_benchmarks!(
	[frame_benchmarking, BaselineBench::<Runtime>]
	[frame_system, SystemBench::<Runtime>]
	[frame_system_extensions, SystemExtensionsBench::<Runtime>]
	[pallet_timestamp, Timestamp]
	[pallet_balances, Balances]
	[pallet_transaction_payment, TransactionPayment]
	[pallet_reward, BlockReward]
	[pallet_difficulty, Difficulty]
	// `pallet_grandpa` stays out. Its benchmarks measure an internal
	// `check_equivocation_proof`, but the Config consumes a handwritten
	// trait built around `report_equivocation`, so a generated file can
	// never implement it. Upstream runtimes wire `()` here too.
	[pallet_validator, Validator]
	[pallet_im_online, ImOnline]
	// `pallet_treasury` stays out. Its payout benchmark spends a hardcoded
	// 100 units, far below this chain's existential deposit, so the transfer
	// can never succeed. Weights stay on the upstream values.
	[pallet_bounties, Bounties]
	// `pallet_child_bounties` stays out. Its setup hardcodes a Root origin for
	// `propose_curator`, but bounty spends here run through OpenGov tracks that
	// Root does not satisfy. Weights stay on the upstream values.
	[pallet_preimage, Preimage]
	[pallet_scheduler, Scheduler]
	[pallet_conviction_voting, ConvictionVoting]
	// `pallet_referenda` stays out. Its setup submits proposals from a Root
	// origin and no track here accepts Root. Weights stay on the upstream
	// values.
	[pallet_multisig, Multisig]
	[pallet_utility, Utility]
	[pallet_proxy, Proxy]
	[pallet_prime, Prime]
	[pallet_vesting, Vesting]
	// `pallet_identity` stays out until its weights get generated. The old
	// blocker is gone now that `ForceOrigin` is `EnsureRoot`, which is what its
	// `kill_username` benchmark dispatches from. Weights stay on the upstream
	// values meanwhile.
);
