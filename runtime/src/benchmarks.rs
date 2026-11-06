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
	[pallet_treasury, Treasury]
	// `pallet_bounties` stays out. Its setup funds the pot with a billion
	// existential deposits. At this chain's one plank ED that falls far short
	// of the bounty it approves, so the bounty never gets funded. Weights stay
	// on the upstream values.
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
	// `pallet_revive` stays out. `seal_call` and `seal_instantiate` send 100
	// units of dust, which needs a `NativeToEthRatio` above 100, and this chain
	// runs at 1. `v2_migration_step` also picks up the genesis code behind
	// `0x…0802` as an extra entry to migrate. Weights stay on the upstream
	// values.
	[pallet_evm_claims, EvmClaims]
	[pallet_identity, Identity]
);
