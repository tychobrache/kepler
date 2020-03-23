// Copyright 2020 The Kepler Developers
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

mod common;
use crate::common::{new_block, tx1i2o, tx2i1o, txspend1i1o};
use crate::core::consensus::BLOCK_OUTPUT_WEIGHT;
use crate::core::core::asset::Asset;
use crate::core::core::block::Error;
use crate::core::core::hash::Hashed;
use crate::core::core::id::ShortIdentifiable;
use crate::core::core::issued_asset::AssetAction;
use crate::core::core::transaction::{self, Error as TxError, Transaction, Weighting};
use crate::core::core::verifier_cache::{LruVerifierCache, VerifierCache};
use crate::core::core::Committed;
use crate::core::core::{
	Block, BlockHeader, CompactBlock, HeaderVersion, KernelFeatures, OutputFeatures,
};
use crate::core::libtx::build::{self, input, output};
use crate::core::libtx::ProofBuilder;
use crate::core::{global, ser};
use chrono::Duration;
use kepler_core as core;
use kepler_core::global::ChainTypes;
use keychain::{BlindingFactor, ExtKeychain, Keychain};
use std::sync::Arc;
use util::secp;
use util::RwLock;

fn verifier_cache() -> Arc<RwLock<dyn VerifierCache>> {
	Arc::new(RwLock::new(LruVerifierCache::new()))
}

#[test]
fn too_large_block() {
	global::set_mining_mode(ChainTypes::AutomatedTesting);
	let asset = Asset::default();
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let max_out = global::max_block_weight() / BLOCK_OUTPUT_WEIGHT;

	let mut pks = vec![];
	for n in 0..(max_out + 1) {
		pks.push(ExtKeychain::derive_key_id(1, n as u32, 0, 0, 0));
	}

	let mut parts = vec![];
	for _ in 0..max_out {
		parts.push(output(5, pks.pop().unwrap()));
	}

	parts.append(&mut vec![input(500000, pks.pop().unwrap())]);
	let tx =
		build::transaction(KernelFeatures::Plain { fee: 2 }, parts, &keychain, &builder).unwrap();

	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(vec![&tx], &keychain, &builder, &prev, &key_id);
	assert!(b
		.validate(&BlindingFactor::zero(), verifier_cache())
		.is_err());
}

#[test]
// block with no inputs/outputs/kernels
// no fees, no reward, no coinbase
fn very_empty_block() {
	let b = Block::with_header(BlockHeader::default());

	assert_eq!(
		b.verify_coinbase(),
		Err(Error::Secp(secp::Error::IncorrectCommitSum))
	);
}

#[test]
// builds a block with a tx spending another and check that cut_through occurred
fn block_with_cut_through() {
	let asset = Asset::default();
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let key_id1 = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let key_id2 = ExtKeychain::derive_key_id(1, 2, 0, 0, 0);
	let key_id3 = ExtKeychain::derive_key_id(1, 3, 0, 0, 0);

	let mut btx1 = tx2i1o();
	let mut btx2 = build::transaction(
		KernelFeatures::Plain { fee: 2 },
		vec![input(7, key_id1), output(5, key_id2.clone())],
		&keychain,
		&builder,
	)
	.unwrap();

	// spending tx2 - reuse key_id2

	let mut btx3 = txspend1i1o(5, &keychain, &builder, key_id2, key_id3);
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(
		vec![&mut btx1, &mut btx2, &mut btx3],
		&keychain,
		&builder,
		&prev,
		&key_id,
	);

	// block should have been automatically compacted (including reward
	// output) and should still be valid
	b.validate(&BlindingFactor::zero(), verifier_cache())
		.unwrap();
	assert_eq!(b.inputs().len(), 3);
	assert_eq!(b.outputs().len(), 3);
}

#[test]
fn empty_block_with_coinbase_is_valid() {
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(vec![], &keychain, &builder, &prev, &key_id);

	assert_eq!(b.inputs().len(), 0);
	assert_eq!(b.outputs().len(), 1);
	assert_eq!(b.kernels().len(), 1);

	let coinbase_outputs = b
		.outputs()
		.iter()
		.filter(|out| out.is_coinbase())
		.cloned()
		.collect::<Vec<_>>();
	assert_eq!(coinbase_outputs.len(), 1);

	let coinbase_kernels = b
		.kernels()
		.iter()
		.filter(|out| out.is_coinbase())
		.cloned()
		.collect::<Vec<_>>();
	assert_eq!(coinbase_kernels.len(), 1);

	// the block should be valid here (single coinbase output with corresponding
	// txn kernel)
	assert!(b
		.validate(&BlindingFactor::zero(), verifier_cache())
		.is_ok());
}

// use std::sync::Arc;
// use crate::util::RwLock;
// use crate::core::verifier_cache::{LruVerifierCache, VerifierCache};

// fn verifier_cache() -> Arc<RwLock<dyn VerifierCache>> {
// 	Arc::new(RwLock::new(LruVerifierCache::new()))
// }
// #[test]
// fn tx_with_duplicate_new_asset() {
// 	let keychain = ExtKeychain::from_random_seed(false).unwrap();
// 	let builder = ProofBuilder::new(&keychain);
// 	let vc = verifier_cache();

// 	let key_id1 = ExtKeychainPath::new(1, 1, 0, 0, 0).to_identifier();
// 	let key_id2 = ExtKeychainPath::new(1, 2, 0, 0, 0).to_identifier();
// 	let key_id3 = ExtKeychainPath::new(1, 3, 0, 0, 0).to_identifier();

// 	let btc_asset: Asset = "BTC".into();

// 	// produce action with the wrong signature
// 	let invalid_action = {
// 		let secp = static_secp_instance();
// 		let secp = secp.lock(); // drop the static lock after using. The same static secp instance is used later in the scope by another function.

// 		let sk = SecretKey::new(&secp, &mut thread_rng());
// 		let pubkey = PublicKey::from_secret_key(&secp, &sk).unwrap();

// 		// Incorrect secret key to sign this action
// 		let sk2 = SecretKey::new(&secp, &mut thread_rng());

// 		let issue_asset = IssuedAsset::new(100, pubkey, false, btc_asset);

// 		let message = Message::from_bytes(&issue_asset.to_bytes()).unwrap();
// 		let sig = secp.sign(&message, &sk2).unwrap();

// 		AssetAction::New(btc_asset, issue_asset, sig)
// 	};

// 	assert!(!invalid_action.validate());

// 	let new_assest_action = {
// 		let secp = static_secp_instance();
// 		let secp = secp.lock(); // drop the static lock after using. The same static secp instance is used later in the scope by another function.

// 		let sk = SecretKey::new(&secp, &mut thread_rng());
// 		//			let sk = SecretKey::from_slice(&secp, &[1; 32]).unwrap();
// 		let pubkey = PublicKey::from_secret_key(&secp, &sk).unwrap();

// 		let issue_asset = IssuedAsset::new(100, pubkey, false, btc_asset);

// 		let message = Message::from_bytes(&issue_asset.to_bytes()).unwrap();
// 		let sig = secp.sign(&message, &sk).unwrap();

// 		AssetAction::New(btc_asset, issue_asset, sig)
// 	};

// 	let new_assest_action2 = {
// 		let secp = static_secp_instance();
// 		let secp = secp.lock(); // drop the static lock after using. The same static secp instance is used later in the scope by another function.

// 		let sk = SecretKey::new(&secp, &mut thread_rng());
// 		//			let sk = SecretKey::from_slice(&secp, &[1; 32]).unwrap();
// 		let pubkey = PublicKey::from_secret_key(&secp, &sk).unwrap();

// 		let issue_asset = IssuedAsset::new(100, pubkey, false, btc_asset);

// 		let message = Message::from_bytes(&issue_asset.to_bytes()).unwrap();
// 		let sig = secp.sign(&message, &sk).unwrap();

// 		AssetAction::New(btc_asset, issue_asset, sig)
// 	};

// 	let badtx = build::transaction(
// 		vec![
// 			input(Asset::default(), 2, key_id1.clone()),
// 			mint(new_assest_action),
// 			mint(new_assest_action2),
// 			output(btc_asset, 100, key_id2.clone()),
// 			with_fee(2),
// 		],
// 		&keychain,
// 		&builder,
// 	)
// 	.unwrap();

// 	match badtx.validate_read() {
// 		Err(transaction::Error::DuplicateAssetPoints) => {}
// 		Err(err) => panic!("unexpected tx error: {}", err),
// 		Ok(()) => panic!("expect tx to be invalid because of duplicate error"),
// 	}

// 	let badtx_badsig = build::transaction(
// 		vec![
// 			input(Asset::default(), 2, key_id1.clone()),
// 			mint(invalid_action),
// 			output(btc_asset, 100, key_id2.clone()),
// 			with_fee(2),
// 		],
// 		&keychain,
// 		&builder,
// 	)
// 	.unwrap();

// 	match badtx_badsig.validate(Weighting::AsTransaction, vc) {
// 		Err(transaction::Error::IncorrectSignature) => {}
// 		Err(err) => panic!("unexpected tx error: {}", err),
// 		Ok(()) => panic!("expect tx to be invalid because of signature error"),
// 	}
// }

// #[test]
// fn block_with_mint_action() {
// 	let keychain = ExtKeychain::from_random_seed(false).unwrap();
// 	let builder = ProofBuilder::new(&keychain);
// 	let prev = BlockHeader::default();
// 	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
// 	let key_id1 = ExtKeychainPath::new(1, 1, 0, 0, 0).to_identifier();
// 	let key_id2 = ExtKeychainPath::new(1, 2, 0, 0, 0).to_identifier();
// 	let key_id3 = ExtKeychainPath::new(1, 3, 0, 0, 0).to_identifier();
// 	let key_id4 = ExtKeychainPath::new(1, 4, 0, 0, 0).to_identifier();
// 	let key_id5 = ExtKeychainPath::new(1, 5, 0, 0, 0).to_identifier();

// 	let vc = verifier_cache();

// 	let btc_asset: Asset = "BTC".into();
// 	// TODO mint fees
// 	// TODO multiple mint outputs

// 	let tx = build::transaction(
// 		vec![
// 			input(Asset::default(), 10, key_id1),
// 			input(Asset::default(), 12, key_id2),
// 			output(Asset::default(), 20, key_id3),
// 			mint(AssetAction::Issue(btc_asset, 100, Default::default())),
// 			output(btc_asset, 50, key_id4),
// 			output(btc_asset, 50, key_id5),
// 			with_fee(2),
// 		],
// 		&keychain,
// 		&builder,
// 	)
// 	.unwrap();

// 	let height = prev.height + 1;

// 	let fees = tx.fee();

// 	let reward_output =
// 		libtx::reward::output(&keychain, &builder, &key_id, fees, height, false).unwrap();
// 	let b =
// 		core::core::Block::new(&prev, vec![tx.clone()], Difficulty::min(), reward_output).unwrap();

// 	// let b = new_block(vec![&tx], &keychain, &builder, &prev, &key_id);

// 	// assert_eq!(b.inputs().len(), 2);
// 	// assert_eq!(b.outputs().len(), 2);
// 	// assert_eq!(b.kernels().len(), 2);

// 	let coinbase_outputs = b
// 		.outputs()
// 		.iter()
// 		.filter(|out| out.is_coinbase())
// 		.map(|o| o.clone())
// 		.collect::<Vec<_>>();
// 	assert_eq!(coinbase_outputs.len(), 1);

// 	let coinbase_kernels = b
// 		.kernels()
// 		.iter()
// 		.filter(|out| out.is_coinbase())
// 		.map(|o| o.clone())
// 		.collect::<Vec<_>>();
// 	assert_eq!(coinbase_kernels.len(), 1);

// 	tx.validate(Weighting::AsTransaction, vc.clone()).unwrap();

// 	// the block should be valid here (single coinbase output with corresponding
// 	// txn kernel)
// 	// match b.validate(&BlindingFactor::zero(), verifier_cache()) {
// 	// 	Err(err) => println!("validate err: {}", err),
// 	// 	Ok(_) => (),
// 	// }

// 	assert!(b
// 		.validate(&BlindingFactor::zero(), verifier_cache())
// 		.is_ok());
// }

#[test]
// test that flipping the COINBASE flag on the output features
// invalidates the block and specifically it causes verify_coinbase to fail
// additionally verifying the merkle_inputs_outputs also fails
fn remove_coinbase_output_flag() {
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let mut b = new_block(vec![], &keychain, &builder, &prev, &key_id);

	assert!(b.outputs()[0].is_coinbase());
	b.outputs_mut()[0].features = OutputFeatures::Plain;

	assert_eq!(b.verify_coinbase(), Err(Error::CoinbaseSumMismatch));
	assert!(b
		.verify_kernel_sums(
			b.header.overage(),
			b.header.issue_overage(),
			b.header.total_kernel_offset(),
		)
		.is_ok());
	assert_eq!(
		b.validate(&BlindingFactor::zero(), verifier_cache()),
		Err(Error::CoinbaseSumMismatch)
	);
}

#[test]
// test that flipping the COINBASE flag on the kernel features
// invalidates the block and specifically it causes verify_coinbase to fail
fn remove_coinbase_kernel_flag() {
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let mut b = new_block(vec![], &keychain, &builder, &prev, &key_id);

	assert!(b.kernels()[0].is_coinbase());
	b.kernels_mut()[0].features = KernelFeatures::Plain { fee: 0 };

	// Flipping the coinbase flag results in kernels not summing correctly.
	assert_eq!(
		b.verify_coinbase(),
		Err(Error::Secp(secp::Error::IncorrectCommitSum))
	);

	// Also results in the block no longer validating correctly
	// because the message being signed on each tx kernel includes the kernel features.
	assert_eq!(
		b.validate(&BlindingFactor::zero(), verifier_cache()),
		Err(Error::Transaction(transaction::Error::IncorrectSignature))
	);
}

#[test]
fn serialize_deserialize_header_version() {
	let mut vec1 = Vec::new();
	ser::serialize_default(&mut vec1, &1_u16).expect("serialization failed");

	let mut vec2 = Vec::new();
	ser::serialize_default(&mut vec2, &HeaderVersion(1)).expect("serialization failed");

	// Check that a header_version serializes to a
	// single u16 value with no extraneous bytes wrapping it.
	assert_eq!(vec1, vec2);

	// Check we can successfully deserialize a header_version.
	let version: HeaderVersion = ser::deserialize_default(&mut &vec2[..]).unwrap();
	assert_eq!(version.0, 1)
}

#[test]
fn serialize_deserialize_block_header() {
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(vec![], &keychain, &builder, &prev, &key_id);
	let header1 = b.header;

	let mut vec = Vec::new();
	ser::serialize_default(&mut vec, &header1).expect("serialization failed");
	let header2: BlockHeader = ser::deserialize_default(&mut &vec[..]).unwrap();

	assert_eq!(header1.hash(), header2.hash());
	assert_eq!(header1, header2);
}

#[test]
fn serialize_deserialize_block() {
	let tx1 = tx1i2o();
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(vec![&tx1], &keychain, &builder, &prev, &key_id);

	let mut vec = Vec::new();
	ser::serialize_default(&mut vec, &b).expect("serialization failed");
	let b2: Block = ser::deserialize_default(&mut &vec[..]).unwrap();

	assert_eq!(b.hash(), b2.hash());
	assert_eq!(b.header, b2.header);
	assert_eq!(b.inputs(), b2.inputs());
	assert_eq!(b.outputs(), b2.outputs());
	assert_eq!(b.kernels(), b2.kernels());
}

#[test]
fn empty_block_serialized_size() {
	global::set_mining_mode(ChainTypes::AutomatedTesting);
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(vec![], &keychain, &builder, &prev, &key_id);
	let mut vec = Vec::new();
	ser::serialize_default(&mut vec, &b).expect("serialization failed");
	let target_len = 1_329;
	assert_eq!(vec.len(), target_len);
}

#[test]
fn block_single_tx_serialized_size() {
	global::set_mining_mode(ChainTypes::AutomatedTesting);
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let tx1 = tx1i2o();
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(vec![&tx1], &keychain, &builder, &prev, &key_id);
	let mut vec = Vec::new();
	ser::serialize_default(&mut vec, &b).expect("serialization failed");
	let target_len = 3_103;
	assert_eq!(vec.len(), target_len);
}

#[test]
fn empty_compact_block_serialized_size() {
	global::set_mining_mode(ChainTypes::AutomatedTesting);
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(vec![], &keychain, &builder, &prev, &key_id);
	let cb: CompactBlock = b.into();
	let mut vec = Vec::new();
	ser::serialize_default(&mut vec, &cb).expect("serialization failed");
	let target_len = 1_337;
	assert_eq!(vec.len(), target_len);
}

#[test]
fn compact_block_single_tx_serialized_size() {
	global::set_mining_mode(ChainTypes::AutomatedTesting);
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let tx1 = tx1i2o();
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(vec![&tx1], &keychain, &builder, &prev, &key_id);
	let cb: CompactBlock = b.into();
	let mut vec = Vec::new();
	ser::serialize_default(&mut vec, &cb).expect("serialization failed");
	let target_len = 1_343;
	assert_eq!(vec.len(), target_len);
}

#[test]
fn block_10_tx_serialized_size() {
	global::set_mining_mode(global::ChainTypes::AutomatedTesting);
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);

	let mut txs = vec![];
	for _ in 0..10 {
		let tx = tx1i2o();
		txs.push(tx);
	}
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(txs.iter().collect(), &keychain, &builder, &prev, &key_id);

	// Default protocol version.
	{
		let mut vec = Vec::new();
		ser::serialize_default(&mut vec, &b).expect("serialization failed");
		assert_eq!(vec.len(), 16_836);
	}

	// Explicit protocol version 1
	{
		let mut vec = Vec::new();
		ser::serialize(&mut vec, ser::ProtocolVersion(1), &b).expect("serialization failed");
		assert_eq!(vec.len(), 16_932);
	}

	// Explicit protocol version 2
	{
		let mut vec = Vec::new();
		ser::serialize(&mut vec, ser::ProtocolVersion(2), &b).expect("serialization failed");
		assert_eq!(vec.len(), 16_836);
	}
	// let mut vec = Vec::new();
	// ser::serialize(&mut vec, &b).expect("serialization failed");
	// let target_len = 19_069;
	// assert_eq!(vec.len(), target_len,);
}

#[test]
fn compact_block_10_tx_serialized_size() {
	global::set_mining_mode(ChainTypes::AutomatedTesting);
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);

	let mut txs = vec![];
	for _ in 0..10 {
		let tx = tx1i2o();
		txs.push(tx);
	}
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(txs.iter().collect(), &keychain, &builder, &prev, &key_id);
	let cb: CompactBlock = b.into();
	let mut vec = Vec::new();
	ser::serialize_default(&mut vec, &cb).expect("serialization failed");
	let target_len = 1_397;
	assert_eq!(vec.len(), target_len,);
}

#[test]
fn compact_block_hash_with_nonce() {
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let tx = tx1i2o();
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(vec![&tx], &keychain, &builder, &prev, &key_id);
	let cb1: CompactBlock = b.clone().into();
	let cb2: CompactBlock = b.clone().into();

	// random nonce will not affect the hash of the compact block itself
	// hash is based on header POW only
	assert!(cb1.nonce != cb2.nonce);
	assert_eq!(b.hash(), cb1.hash());
	assert_eq!(cb1.hash(), cb2.hash());

	assert!(cb1.kern_ids()[0] != cb2.kern_ids()[0]);

	// check we can identify the specified kernel from the short_id
	// correctly in both of the compact_blocks
	assert_eq!(
		cb1.kern_ids()[0],
		tx.kernels()[0].short_id(&cb1.hash(), cb1.nonce)
	);
	assert_eq!(
		cb2.kern_ids()[0],
		tx.kernels()[0].short_id(&cb2.hash(), cb2.nonce)
	);
}

#[test]
fn convert_block_to_compact_block() {
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let tx1 = tx1i2o();
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(vec![&tx1], &keychain, &builder, &prev, &key_id);
	let cb: CompactBlock = b.clone().into();

	assert_eq!(cb.out_full().len(), 1);
	assert_eq!(cb.kern_full().len(), 1);
	assert_eq!(cb.kern_ids().len(), 1);

	assert_eq!(
		cb.kern_ids()[0],
		b.kernels()
			.iter()
			.find(|x| !x.is_coinbase())
			.unwrap()
			.short_id(&cb.hash(), cb.nonce)
	);
}

#[test]
fn hydrate_empty_compact_block() {
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(vec![], &keychain, &builder, &prev, &key_id);
	let cb: CompactBlock = b.clone().into();
	let hb = Block::hydrate_from(cb, vec![]).unwrap();
	assert_eq!(hb.header, b.header);
	assert_eq!(hb.outputs(), b.outputs());
	assert_eq!(hb.kernels(), b.kernels());
}

#[test]
fn serialize_deserialize_compact_block() {
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let tx1 = tx1i2o();
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(vec![&tx1], &keychain, &builder, &prev, &key_id);

	let mut cb1: CompactBlock = b.into();

	let mut vec = Vec::new();
	ser::serialize_default(&mut vec, &cb1).expect("serialization failed");

	// After header serialization, timestamp will lose 'nanos' info, that's the designed behavior.
	// To suppress 'nanos' difference caused assertion fail, we force b.header also lose 'nanos'.
	let origin_ts = cb1.header.timestamp;
	cb1.header.timestamp =
		origin_ts - Duration::nanoseconds(origin_ts.timestamp_subsec_nanos() as i64);

	let cb2: CompactBlock = ser::deserialize_default(&mut &vec[..]).unwrap();

	assert_eq!(cb1.header, cb2.header);
	assert_eq!(cb1.kern_ids(), cb2.kern_ids());
}

// Duplicate a range proof from a valid output into another of the same amount
#[test]
fn same_amount_outputs_copy_range_proof() {
	let asset = Asset::default();
	let keychain = keychain::ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let key_id1 = keychain::ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let key_id2 = keychain::ExtKeychain::derive_key_id(1, 2, 0, 0, 0);
	let key_id3 = keychain::ExtKeychain::derive_key_id(1, 3, 0, 0, 0);

	let tx = build::transaction(
		KernelFeatures::Plain { fee: 1 },
		vec![input(7, key_id1), output(3, key_id2), output(3, key_id3)],
		&keychain,
		&builder,
	)
	.unwrap();

	// now we reconstruct the transaction, swapping the rangeproofs so they
	// have the wrong privkey
	let ins = tx.inputs();
	let mut outs = tx.outputs().clone();
	let kernels = tx.kernels();
	outs[0].proof = outs[1].proof;

	let key_id = keychain::ExtKeychain::derive_key_id(1, 4, 0, 0, 0);
	let prev = BlockHeader::default();
	let b = new_block(
		vec![&mut Transaction::new(
			ins.clone(),
			outs,
			kernels.clone(),
			Vec::new(),
		)],
		&keychain,
		&builder,
		&prev,
		&key_id,
	);

	// block should have been automatically compacted (including reward
	// output) and should still be valid
	match b.validate(&BlindingFactor::zero(), verifier_cache()) {
		Err(Error::Transaction(transaction::Error::Secp(secp::Error::InvalidRangeProof))) => {}
		_ => panic!("Bad range proof should be invalid"),
	}
}

// Swap a range proof with the right private key but wrong amount
#[test]
fn wrong_amount_range_proof() {
	let asset = Asset::default();
	let keychain = keychain::ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let key_id1 = keychain::ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let key_id2 = keychain::ExtKeychain::derive_key_id(1, 2, 0, 0, 0);
	let key_id3 = keychain::ExtKeychain::derive_key_id(1, 3, 0, 0, 0);

	let tx1 = build::transaction(
		KernelFeatures::Plain { fee: 1 },
		vec![
			input(7, key_id1.clone()),
			output(3, key_id2.clone()),
			output(3, key_id3.clone()),
		],
		&keychain,
		&builder,
	)
	.unwrap();
	let tx2 = build::transaction(
		KernelFeatures::Plain { fee: 1 },
		vec![input(7, key_id1), output(2, key_id2), output(4, key_id3)],
		&keychain,
		&builder,
	)
	.unwrap();

	// we take the range proofs from tx2 into tx1 and rebuild the transaction
	let ins = tx1.inputs();
	let mut outs = tx1.outputs().clone();
	let kernels = tx1.kernels();
	outs[0].proof = tx2.outputs()[0].proof;
	outs[1].proof = tx2.outputs()[1].proof;

	let key_id = keychain::ExtKeychain::derive_key_id(1, 4, 0, 0, 0);
	let prev = BlockHeader::default();
	let b = new_block(
		vec![&mut Transaction::new(
			ins.clone(),
			outs,
			kernels.clone(),
			Vec::new(),
		)],
		&keychain,
		&builder,
		&prev,
		&key_id,
	);

	// block should have been automatically compacted (including reward
	// output) and should still be valid
	match b.validate(&BlindingFactor::zero(), verifier_cache()) {
		Err(Error::Transaction(transaction::Error::Secp(secp::Error::InvalidRangeProof))) => {}
		_ => panic!("Bad range proof should be invalid"),
	}
}

#[test]
fn validate_header_proof() {
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(vec![], &keychain, &builder, &prev, &key_id);

	let mut header_buf = vec![];
	{
		let mut writer = ser::BinWriter::default(&mut header_buf);
		b.header.write_pre_pow(&mut writer).unwrap();
		b.header.pow.write_pre_pow(&mut writer).unwrap();
	}
	let pre_pow = util::to_hex(header_buf);

	let reconstructed = BlockHeader::from_pre_pow_and_proof(
		pre_pow,
		b.header.pow.nonce,
		b.header.pow.proof.clone(),
	)
	.unwrap();
	assert_eq!(reconstructed, b.header);

	// assert invalid pre_pow returns error
	assert!(BlockHeader::from_pre_pow_and_proof(
		"0xaf1678".to_string(),
		b.header.pow.nonce,
		b.header.pow.proof,
	)
	.is_err());
}
