// Copyright 2018 The Kepler Developers
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use self::chain::types::NoopAdapter;
use self::chain::ErrorKind;
use self::chain::{Chain, Error};
use self::core::core::verifier_cache::LruVerifierCache;
use self::core::global::{self, ChainTypes};
use self::core::libtx::{self, build, ProofBuilder};
use self::core::pow::Difficulty;
use self::core::{consensus, pow};
use self::keychain::{ExtKeychain, ExtKeychainPath, Identifier, Keychain};
use self::util::RwLock;
use chrono::Duration;
use env_logger;
use kepler_chain as chain;
use kepler_core as core;
use kepler_core::core::{Block, Transaction, Committed, OutputIdentifier};
use kepler_keychain as keychain;
use kepler_util as util;
use std::fs;
use std::sync::Arc;

fn clean_output_dir(dir_name: &str) {
	let _ = fs::remove_dir_all(dir_name);
}

// 1. mine a spendable output
// 2. issue an asset
// 3. check block headers for correctness

struct Harness<'a> {
	chain: Chain,
	keychain: &'a ExtKeychain,
	builder: ProofBuilder<'a, ExtKeychain>,
	d0: u32,
}

impl<'a> Harness<'a> {
	fn setup(chain_dir: &str, keychain: &'a ExtKeychain) -> Harness<'a> {
//		let _ = env_logger::init();
		match env_logger::try_init() {
			Ok(_) => println!("Initializing env logger"),
			_ => {}
//			Err(e) => println!("env logger already initialized: {:?}", e),
		};


		clean_output_dir(chain_dir);
		global::set_mining_mode(ChainTypes::AutomatedTesting);

		let verifier_cache = Arc::new(RwLock::new(LruVerifierCache::new()));
		let genesis_block = pow::mine_genesis_block().unwrap();

		let chain = Chain::init(
			chain_dir.to_string(),
			Arc::new(NoopAdapter {}),
			genesis_block,
			pow::verify_size,
			verifier_cache,
			false,
		)
		.unwrap();

		let builder = ProofBuilder::new(keychain);

		Harness {
			chain,
			keychain,
			builder,
			d0: 0,
		}
	}

	fn build_block(&mut self, txs: Vec<Transaction>, reward_output_key_id: Identifier) -> Block {
		let prev = self.chain.head_header().unwrap();
		let fees = txs.iter().map(|tx| tx.fee()).sum();
		let height = prev.height + 1;
		let next_header_info =
			consensus::next_difficulty(height, self.chain.difficulty_iter().unwrap());
		let reward = libtx::reward::output(
			self.keychain,
			&self.builder,
			&reward_output_key_id,
			fees,
			height,
			false,
		)
		.unwrap();
		let mut block = core::core::Block::new(&prev, txs, Difficulty::min(), reward).unwrap();
		block.header.timestamp = prev.timestamp + Duration::seconds(60);
		block.header.pow.secondary_scaling = next_header_info.secondary_scaling;

		// Set fields on the block header by applying the transaction. Rollback after, keeping the chain itself unmodified.
		self.chain.set_txhashset_roots(&mut block).unwrap();

		pow::pow_size(
			&mut block.header,
			next_header_info.difficulty,
			global::proofsize(),
			global::min_edge_bits(),
		)
		.unwrap();

		block
	}

	fn mine_block(&mut self, txs: Vec<Transaction>) -> (Block, Identifier) {
		let reward_output_key_id = self.next_key_id();

		let block = self.build_block(txs, reward_output_key_id.clone());

		self.chain
			.process_block(block.clone(), chain::Options::MINE)
			.unwrap();

		(block, reward_output_key_id)
	}

	fn mine_empty_block(&mut self) -> (Block, Identifier) {
		let (block, key_id) = self.mine_block(vec![]);

		assert_eq!(block.outputs().len(), 1);
		let coinbase_output = block.outputs()[0];
		assert!(coinbase_output.is_coinbase());

		(block, key_id)
	}

	fn next_key_id(&mut self) -> Identifier {
		self.d0 += 1;
		ExtKeychainPath::new(1, self.d0, 0, 0, 0).to_identifier()
	}

	// Assuming that our tests will not exceed the halving period in height, so all the rewards
	// would just be the same.
	fn reward_amount(&mut self) -> u64 {
		consensus::reward(1, 0)
	}

	fn build_spend_coinbase_tx(&mut self, coinbase_input: Identifier) -> (Transaction, Identifier) {
		// This seems only incidentally right... because in the test we don't exceed the halving interval, so all the rewards are the same for our differing heights.
		let prev = self.chain.head_header().unwrap();
		let height = prev.height + 1;
		let amount = consensus::reward(height, 0);

		let plain_output_keyid = self.next_key_id();

		let coinbase_txn = build::transaction(
			vec![
				build::coinbase_input(amount, coinbase_input),
				build::output(Default::default(), amount - 2, plain_output_keyid.clone()),
				build::with_fee(2),
			],
			self.keychain,
			&self.builder,
		)
		.unwrap();

		(coinbase_txn, plain_output_keyid)
	}

	fn build_spend_plain_output_tx(&mut self, plain_utxo_keyid: Identifier) -> (Transaction, Identifier) {
		let output_keyid = self.next_key_id();

		// kinda ugly... we expect the plain output to be 2 less than the coinbase input.
		// FIXME: is there a way to figure out the size of an output given keychain?
		let amount = self.reward_amount() - 2;

		// how much should the fees be?
		let tx = build::transaction(
			vec![
				build::input(Default::default(), amount, plain_utxo_keyid),
				build::output(Default::default(), amount - 2, output_keyid.clone()),
				build::with_fee(2),
			],
			self.keychain,
			&self.builder,
		)
			.unwrap();

		(tx, output_keyid)
	}

	// Converting a mature coinbase input to a plain output
	fn spend_coinbase(&mut self, coinbase_input: Identifier) -> (Block, Identifier) {
		let (spend_coinbase_tx, plain_output_keyid) = self.build_spend_coinbase_tx(coinbase_input.clone());
		let (block, _) = self.mine_block(vec![spend_coinbase_tx]);
		(block, plain_output_keyid)
	}

	fn verify_coinbase_maturity(&mut self, coinbase_input: Identifier) -> Result<(), Error> {
		let (coinbase_txn, _) = self.build_spend_coinbase_tx(coinbase_input);

		let prev = self.chain.head_header().unwrap();
		let height = prev.height + 1;

		// Question: seems pointless to build a block here
		{
			let reward_key_id = self.next_key_id();
			let txs = vec![coinbase_txn.clone()];
			let block = self.build_block(txs, reward_key_id.clone());
		}

		self.chain.verify_coinbase_maturity(&coinbase_txn)
	}

	// Confirm the tx attempting to spend the coinbase output
	// is not valid at the current block height given the current chain state.
	fn expect_immature_coinbase(&mut self, coinbase_input: Identifier) {
		match self.verify_coinbase_maturity(coinbase_input) {
			Ok(_) => {}
			Err(e) => match e.kind() {
				ErrorKind::ImmatureCoinbase => {}
				_ => panic!("Expected transaction error with immature coinbase."),
			},
		}
	}

	fn expect_mature_coinbase(&mut self, coinbase_input: Identifier) {
		match self.verify_coinbase_maturity(coinbase_input) {
			Ok(_) => {}
			Err(e) => panic!("Expected mature coinbase: {}", e),
		}
	}

	// Mine a plain input, by spending a matured coinbase
	fn mine_plain_output(&mut self) -> (Block, Identifier) {
		let (_, reward) = self.mine_empty_block();

		for _ in 0..3 {
			self.mine_empty_block();
		}

		let (block, output_keyid) = self.spend_coinbase(reward);


		(block, output_keyid)
	}
}

#[test]
fn test_plain_output_spendable() {
	let chain_dir = ".kepler_test_plain_output_spendable";

	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let mut h = Harness::setup(chain_dir, &keychain);

	let (block, output) = h.mine_plain_output();

	match h.chain.is_unspent(&block.outputs()[1].into()) {
		Ok(_) => {},
		Err(err) => {
			panic!("output is not found (or already spent): {}", err);
		}
	}

	let (tx, _) = h.build_spend_plain_output_tx(output);

	match h.chain.validate_tx(&tx) {
		Err(err) => {
			panic!("cannot validate spending tx: {}", err);
		},
		Ok(_) => {},
	};
}

#[test]
fn test_issue_assets() {
	let chain_dir = ".kepler_test_issue_assets";

	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let mut h = Harness::setup(chain_dir, &keychain);

	let lock_height = 1 + global::coinbase_maturity();
	assert_eq!(lock_height, 4);

	{
		let (_, reward) = h.mine_empty_block();
		h.expect_immature_coinbase(reward.clone());
	}

	// Do the same test 3 times, and spending the coinbase after mining 3 blocks to mature it.
	for _ in 0..3 {
		let (_, reward) = h.mine_empty_block();

		h.expect_immature_coinbase(reward.clone());

		for _ in 0..3 {
			h.mine_empty_block();
		}

		h.expect_mature_coinbase(reward.clone());

		h.spend_coinbase(reward.clone());
	}

	// Cleanup chain directory
	clean_output_dir(chain_dir);
}
