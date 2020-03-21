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

//! Blocks and blockheaders

use crate::consensus::{self, reward};
use crate::core::committed::{self, Committed};
use crate::core::compact_block::{CompactBlock, CompactBlockBody};
use crate::core::hash::{DefaultHashable, Hash, Hashed, ZERO_HASH};
use crate::core::verifier_cache::VerifierCache;
use crate::core::{
	asset::Asset, transaction, Commitment, Input, KernelFeatures, Output, Transaction,
	TransactionBody, TxKernel, Weighting,
};
use crate::global;
use crate::pow::{verify_size, Difficulty, Proof, ProofOfWork};
use crate::ser::{
	self, deserialize_default, serialize_default, PMMRable, Readable, Reader, Writeable, Writer,
};
use chrono::naive::{MAX_DATE, MIN_DATE};
use chrono::prelude::{DateTime, NaiveDateTime, Utc};
use chrono::Duration;
use keychain::{self, BlindingFactor};
use std::collections::HashSet;
use std::convert::TryInto;
use std::fmt;
use std::iter::FromIterator;
use std::sync::Arc;
use util::from_hex;
use util::RwLock;
use util::{secp, static_secp_instance};

use super::issued_asset::AssetAction;

lazy_static! {
	/// The "zero" overage when no asset had been issued. This is loading 32 zero-bytes as generator.
	/// [0u8;32]*G isn't the infinity/zero point.
	pub static ref ZERO_OVERAGE_COMMITMENT: Commitment = {
		let secp = static_secp_instance();
		let secp = secp.lock();
		secp.commit_value_with_generator(0, Asset::default().into()).unwrap()
	};
}

/// Errors thrown by Block validation
#[derive(Debug, Clone, Eq, PartialEq, Fail)]
pub enum Error {
	/// The sum of output minus input commitments does not
	/// match the sum of kernel commitments
	KernelSumMismatch,
	/// The total kernel sum on the block header is wrong
	InvalidTotalKernelSum,
	/// Same as above but for the coinbase part of a block, including reward
	CoinbaseSumMismatch,
	/// Restrict block total weight.
	TooHeavy,
	/// Block weight (based on inputs|outputs|kernels) exceeded.
	WeightExceeded,
	/// Block version is invalid for a given block height
	InvalidBlockVersion(HeaderVersion),
	/// Block time is invalid
	InvalidBlockTime,
	/// Invalid POW
	InvalidPow,
	/// Kernel not valid due to lock_height exceeding block header height
	KernelLockHeight(u64),
	/// Underlying tx related error
	Transaction(transaction::Error),
	/// Underlying Secp256k1 error (signature validation or invalid public key
	/// typically)
	Secp(secp::Error),
	/// Underlying keychain related error
	Keychain(keychain::Error),
	/// Underlying Merkle proof error
	MerkleProof,
	/// Error when verifying kernel sums via committed trait.
	Committed(committed::Error),
	/// Validation error relating to cut-through.
	/// Specifically the tx is spending its own output, which is not valid.
	CutThrough,
	/// Underlying serialization error.
	Serialization(ser::Error),
	/// Other unspecified error condition
	Other(String),
}

impl From<committed::Error> for Error {
	fn from(e: committed::Error) -> Error {
		Error::Committed(e)
	}
}

impl From<transaction::Error> for Error {
	fn from(e: transaction::Error) -> Error {
		Error::Transaction(e)
	}
}

impl From<ser::Error> for Error {
	fn from(e: ser::Error) -> Error {
		Error::Serialization(e)
	}
}

impl From<secp::Error> for Error {
	fn from(e: secp::Error) -> Error {
		Error::Secp(e)
	}
}

impl From<keychain::Error> for Error {
	fn from(e: keychain::Error) -> Error {
		Error::Keychain(e)
	}
}

impl fmt::Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "Block Error (display needs implementation")
	}
}

/// Header entry for storing in the header MMR.
/// Note: we hash the block header itself and maintain the hash in the entry.
/// This allows us to lookup the original header from the db as necessary.
#[derive(Debug)]
pub struct HeaderEntry {
	hash: Hash,
	timestamp: u64,
	total_difficulty: Difficulty,
	secondary_scaling: u32,
	is_secondary: bool,
}

impl Readable for HeaderEntry {
	fn read(reader: &mut dyn Reader) -> Result<HeaderEntry, ser::Error> {
		let hash = Hash::read(reader)?;
		let timestamp = reader.read_u64()?;
		let total_difficulty = Difficulty::read(reader)?;
		let secondary_scaling = reader.read_u32()?;

		// Using a full byte to represent the bool for now.
		let is_secondary = reader.read_u8()? != 0;

		Ok(HeaderEntry {
			hash,
			timestamp,
			total_difficulty,
			secondary_scaling,
			is_secondary,
		})
	}
}

impl Writeable for HeaderEntry {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		self.hash.write(writer)?;
		writer.write_u64(self.timestamp)?;
		self.total_difficulty.write(writer)?;
		writer.write_u32(self.secondary_scaling)?;

		// Using a full byte to represent the bool for now.
		if self.is_secondary {
			writer.write_u8(1)?;
		} else {
			writer.write_u8(0)?;
		}
		Ok(())
	}
}

impl Hashed for HeaderEntry {
	/// The hash of the underlying block.
	fn hash(&self) -> Hash {
		self.hash
	}
}

/// Some type safety around header versioning.
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Serialize)]
pub struct HeaderVersion(pub u16);

impl From<HeaderVersion> for u16 {
	fn from(v: HeaderVersion) -> u16 {
		v.0
	}
}

impl Writeable for HeaderVersion {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_u16(self.0)
	}
}

impl Readable for HeaderVersion {
	fn read(reader: &mut dyn Reader) -> Result<HeaderVersion, ser::Error> {
		let version = reader.read_u16()?;
		Ok(HeaderVersion(version))
	}
}

/// Block header, fairly standard compared to other blockchains.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BlockHeader {
	/// Version of the block
	pub version: HeaderVersion,
	/// Height of this block since the genesis block (height 0)
	pub height: u64,
	/// Hash of the block previous to this in the chain.
	pub prev_hash: Hash,
	/// Root hash of the header MMR at the previous header.
	pub prev_root: Hash,
	/// Timestamp at which the block was built.
	pub timestamp: DateTime<Utc>,
	/// Merklish root of all the commitments in the TxHashSet
	pub output_root: Hash,
	/// Merklish root of all range proofs in the TxHashSet
	pub range_proof_root: Hash,
	/// Merklish root of all transaction kernels in the TxHashSet
	pub kernel_root: Hash,
	/// Merklish root of all assets in the TxHashSet
	pub issue_root: Hash,
	/// Total accumulated sum of kernel offsets since genesis block.
	/// We can derive the kernel offset sum for *this* block from
	/// the total kernel offset of the previous block header.
	pub total_kernel_offset: BlindingFactor,
	/// Total size of the output MMR after applying this block
	pub output_mmr_size: u64,
	/// Total size of the kernel MMR after applying this block
	pub kernel_mmr_size: u64,

	/// Total size of the issued assets MMR after applying this block
	pub issue_mmr_size: u64,

	/// Total asset issue/create, as the sum of outputs
	pub total_issue_overage: Commitment,

	/// Proof of work and related
	pub pow: ProofOfWork,
}
impl DefaultHashable for BlockHeader {}

impl Default for BlockHeader {
	fn default() -> BlockHeader {
		BlockHeader {
			version: HeaderVersion(1),
			height: 0,
			timestamp: DateTime::<Utc>::from_utc(NaiveDateTime::from_timestamp(0, 0), Utc),
			prev_hash: ZERO_HASH,
			prev_root: ZERO_HASH,
			output_root: ZERO_HASH,
			range_proof_root: ZERO_HASH,
			kernel_root: ZERO_HASH,
			issue_root: ZERO_HASH,
			total_kernel_offset: BlindingFactor::zero(),
			total_issue_overage: *ZERO_OVERAGE_COMMITMENT,
			output_mmr_size: 0,
			kernel_mmr_size: 0,
			issue_mmr_size: 0,
			pow: ProofOfWork::default(),
		}
	}
}

impl PMMRable for BlockHeader {
	type E = HeaderEntry;

	fn as_elmt(&self) -> Self::E {
		HeaderEntry {
			hash: self.hash(),
			timestamp: self.timestamp.timestamp() as u64,
			total_difficulty: self.total_difficulty(),
			secondary_scaling: self.pow.secondary_scaling,
			is_secondary: self.pow.is_secondary(),
		}
	}

	// Size is hash + u64 + difficulty + u32 + u8.
	fn elmt_size() -> Option<u16> {
		const LEN: usize = Hash::LEN + 8 + Difficulty::LEN + 4 + 1;
		Some(LEN.try_into().unwrap())
	}
}

/// Serialization of a block header
impl Writeable for BlockHeader {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		if writer.serialization_mode() != ser::SerializationMode::Hash {
			self.write_pre_pow(writer)?;
		}
		self.pow.write(writer)?;
		Ok(())
	}
}

fn read_block_header(reader: &mut dyn Reader) -> Result<BlockHeader, ser::Error> {
	let version = HeaderVersion::read(reader)?;
	let (height, timestamp) = ser_multiread!(reader, read_u64, read_i64);
	let prev_hash = Hash::read(reader)?;
	let prev_root = Hash::read(reader)?;
	let output_root = Hash::read(reader)?;
	let range_proof_root = Hash::read(reader)?;
	let kernel_root = Hash::read(reader)?;
	let issue_root = Hash::read(reader)?;
	let total_kernel_offset = BlindingFactor::read(reader)?;
	let (output_mmr_size, kernel_mmr_size, issue_mmr_size) =
		ser_multiread!(reader, read_u64, read_u64, read_u64);
	let total_issue_overage = Commitment::read(reader)?;
	let pow = ProofOfWork::read(reader)?;

	if timestamp > MAX_DATE.and_hms(0, 0, 0).timestamp()
		|| timestamp < MIN_DATE.and_hms(0, 0, 0).timestamp()
	{
		return Err(ser::Error::CorruptedData);
	}

	Ok(BlockHeader {
		version,
		height,
		timestamp: DateTime::<Utc>::from_utc(NaiveDateTime::from_timestamp(timestamp, 0), Utc),
		prev_hash,
		prev_root,
		output_root,
		range_proof_root,
		kernel_root,
		issue_root,
		total_kernel_offset,
		output_mmr_size,
		kernel_mmr_size,
		issue_mmr_size,
		total_issue_overage,
		pow,
	})
}

/// Deserialization of a block header
impl Readable for BlockHeader {
	fn read(reader: &mut dyn Reader) -> Result<BlockHeader, ser::Error> {
		read_block_header(reader)
	}
}

impl BlockHeader {
	/// Add a commitment to header's issue overage
	pub fn add_issue_overage(&self, issue_overage: Commitment) -> Result<Commitment, Error> {
		let new_overage = if self.total_issue_overage == *ZERO_OVERAGE_COMMITMENT {
			issue_overage
		} else {
			let secp = static_secp_instance();
			let secp = secp.lock();
			secp.commit_sum(vec![self.total_issue_overage, issue_overage], vec![])?
		};

		return Ok(new_overage);
	}

	/// Write the pre-hash portion of the header
	pub fn write_pre_pow<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		self.version.write(writer)?;
		ser_multiwrite!(
			writer,
			[write_u64, self.height],
			[write_i64, self.timestamp.timestamp()],
			[write_fixed_bytes, &self.prev_hash],
			[write_fixed_bytes, &self.prev_root],
			[write_fixed_bytes, &self.output_root],
			[write_fixed_bytes, &self.range_proof_root],
			[write_fixed_bytes, &self.kernel_root],
			[write_fixed_bytes, &self.issue_root],
			[write_fixed_bytes, &self.total_kernel_offset],
			[write_u64, self.output_mmr_size],
			[write_u64, self.kernel_mmr_size],
			[write_u64, self.issue_mmr_size]
		);
		self.total_issue_overage.write(writer)?;
		Ok(())
	}

	/// Return the pre-pow, unhashed
	/// Let the cuck(at)oo miner/verifier handle the hashing
	/// for consistency with how this call is performed everywhere
	/// else
	pub fn pre_pow(&self) -> Vec<u8> {
		let mut header_buf = vec![];
		{
			let mut writer = ser::BinWriter::default(&mut header_buf);
			self.write_pre_pow(&mut writer).unwrap();
			self.pow.write_pre_pow(&mut writer).unwrap();
			writer.write_u64(self.pow.nonce).unwrap();
		}
		header_buf
	}

	/// Constructs a header given pre_pow string, nonce, and proof
	pub fn from_pre_pow_and_proof(
		pre_pow: String,
		nonce: u64,
		proof: Proof,
	) -> Result<Self, Error> {
		// Convert hex pre pow string
		let mut header_bytes = from_hex(pre_pow)
			.map_err(|e| Error::Serialization(ser::Error::HexError(e.to_string())))?;
		// Serialize and append serialized nonce and proof
		serialize_default(&mut header_bytes, &nonce)?;
		serialize_default(&mut header_bytes, &proof)?;

		// Deserialize header from constructed bytes
		Ok(deserialize_default(&mut &header_bytes[..])?)
	}

	/// Total difficulty accumulated by the proof of work on this header
	pub fn total_difficulty(&self) -> Difficulty {
		self.pow.total_difficulty
	}

	/// The "overage" to use when verifying the kernel sums.
	/// For a block header the overage is 0 - reward.
	pub fn overage(&self) -> i64 {
		(reward(self.height, 0) as i64).checked_neg().unwrap_or(0)
	}

	/// The "total overage" to use when verifying the kernel sums for a full
	/// chain state. For a full chain state this is 0 - (height * reward).
	pub fn total_overage(&self, genesis_had_reward: bool) -> i64 {
		let mut total_rewards = 0;
		if genesis_had_reward {
			total_rewards += reward(0, 0);
		}
		for height in 1..=self.height {
			total_rewards += reward(height, 0);
		}
		(total_rewards as i64).checked_neg().unwrap_or(0)
	}

	/// Total kernel offset for the chain state up to and including this block.
	pub fn total_kernel_offset(&self) -> BlindingFactor {
		self.total_kernel_offset.clone()
	}
}

impl From<UntrustedBlockHeader> for BlockHeader {
	fn from(header: UntrustedBlockHeader) -> Self {
		header.0
	}
}

/// Block header which does lightweight validation as part of deserialization,
/// it supposed to be used when we can't trust the channel (eg network)
pub struct UntrustedBlockHeader(BlockHeader);

/// Deserialization of an untrusted block header
impl Readable for UntrustedBlockHeader {
	fn read(reader: &mut dyn Reader) -> Result<UntrustedBlockHeader, ser::Error> {
		let header = read_block_header(reader)?;
		if header.timestamp
			> Utc::now() + Duration::seconds(12 * (consensus::BLOCK_TIME_SEC as i64))
		{
			// refuse blocks more than 12 blocks intervals in future (as in bitcoin)
			// TODO add warning in p2p code if local time is too different from peers
			error!(
				"block header {} validation error: block time is more than 12 blocks in future",
				header.hash()
			);
			return Err(ser::Error::CorruptedData);
		}

		// Check the block version before proceeding any further.
		// We want to do this here because blocks can be pretty large
		// and we want to halt processing as early as possible.
		// If we receive an invalid block version then the peer is not on our hard-fork.
		if !consensus::valid_header_version(header.height, header.version) {
			return Err(ser::Error::InvalidBlockVersion);
		}

		if !header.pow.is_primary() && !header.pow.is_secondary() {
			error!(
				"block header {} validation error: invalid edge bits",
				header.hash()
			);
			return Err(ser::Error::CorruptedData);
		}
		if let Err(e) = verify_size(&header) {
			error!(
				"block header {} validation error: invalid POW: {}",
				header.hash(),
				e
			);
			return Err(ser::Error::CorruptedData);
		}
		Ok(UntrustedBlockHeader(header))
	}
}

/// A block as expressed in the Mimblewimble protocol. The reward is
/// non-explicit, assumed to be deducible from block height (similar to
/// bitcoin's schedule) and expressed as a global transaction fee (added v.H),
/// additive to the total of fees ever collected.
#[derive(Debug, Clone, Serialize)]
pub struct Block {
	/// The header with metadata and commitments to the rest of the data
	pub header: BlockHeader,
	/// The body - inputs/outputs/kernels
	body: TransactionBody,
}

impl Hashed for Block {
	/// The hash of the underlying block.
	fn hash(&self) -> Hash {
		self.header.hash()
	}
}

/// Implementation of Writeable for a block, defines how to write the block to a
/// binary writer. Differentiates between writing the block for the purpose of
/// full serialization and the one of just extracting a hash.
impl Writeable for Block {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		self.header.write(writer)?;

		if writer.serialization_mode() != ser::SerializationMode::Hash {
			self.body.write(writer)?;
		}
		Ok(())
	}
}

/// Implementation of Readable for a block, defines how to read a full block
/// from a binary stream.
impl Readable for Block {
	fn read(reader: &mut dyn Reader) -> Result<Block, ser::Error> {
		let header = BlockHeader::read(reader)?;
		let body = TransactionBody::read(reader)?;
		Ok(Block { header, body })
	}
}

/// Provides all information from a block that allows the calculation of total
/// Pedersen commitment.
impl Committed for Block {
	fn inputs_committed(&self) -> Vec<Commitment> {
		self.body.inputs_committed()
	}

	fn outputs_committed(&self) -> Vec<Commitment> {
		self.body.outputs_committed()
	}

	fn kernels_committed(&self) -> Vec<Commitment> {
		self.body.kernels_committed()
	}
}

/// Default properties for a block, everything zeroed out and empty vectors.
impl Default for Block {
	fn default() -> Block {
		Block {
			header: Default::default(),
			body: Default::default(),
		}
	}
}

impl Block {
	/// Builds a new block from the header of the previous block, a vector of
	/// transactions and the private key that will receive the reward. Checks
	/// that all transactions are valid and calculates the Merkle tree.
	///
	/// TODO - Move this somewhere where only tests will use it.
	/// *** Only used in tests. ***
	///
	#[warn(clippy::new_ret_no_self)]
	pub fn new(
		prev: &BlockHeader,
		txs: Vec<Transaction>,
		difficulty: Difficulty,
		reward_output: (Output, TxKernel),
	) -> Result<Block, Error> {
		let mut block =
			Block::from_reward(prev, txs, reward_output.0, reward_output.1, difficulty)?;

		// Now set the pow on the header so block hashing works as expected.
		{
			let proof_size = global::proofsize();
			block.header.pow.proof = Proof::random(proof_size);
		}

		Ok(block)
	}

	/// Hydrate a block from a compact block.
	/// Note: caller must validate the block themselves, we do not validate it
	/// here.
	pub fn hydrate_from(cb: CompactBlock, txs: Vec<Transaction>) -> Result<Block, Error> {
		trace!("block: hydrate_from: {}, {} txs", cb.hash(), txs.len(),);

		let header = cb.header.clone();

		let mut all_inputs = HashSet::new();
		let mut all_outputs = HashSet::new();
		let mut all_kernels = HashSet::new();
		let mut all_assets = HashSet::new();

		// collect all the inputs, outputs and kernels from the txs
		for tx in txs {
			let tb: TransactionBody = tx.into();
			all_inputs.extend(tb.inputs);
			all_outputs.extend(tb.outputs);
			all_kernels.extend(tb.kernels);
			all_assets.extend(tb.assets);
		}

		// include the coinbase output(s) and kernel(s) from the compact_block
		{
			let body: CompactBlockBody = cb.into();
			all_outputs.extend(body.out_full);
			all_kernels.extend(body.kern_full);
		}

		// convert the sets to vecs
		let all_inputs = Vec::from_iter(all_inputs);
		let all_outputs = Vec::from_iter(all_outputs);
		let all_kernels = Vec::from_iter(all_kernels);
		let all_assets = Vec::from_iter(all_assets);

		// Initialize a tx body and sort everything.
		let body = TransactionBody::init(all_inputs, all_outputs, all_kernels, all_assets, false)?;

		// Finally return the full block.
		// Note: we have not actually validated the block here,
		// caller must validate the block.
		Block { header, body }.cut_through()
	}

	/// Build a new empty block from a specified header
	pub fn with_header(header: BlockHeader) -> Block {
		Block {
			header,
			..Default::default()
		}
	}

	/// Builds a new block ready to mine from the header of the previous block,
	/// a vector of transactions and the reward information. Checks
	/// that all transactions are valid and calculates the Merkle tree.
	pub fn from_reward(
		prev: &BlockHeader,
		txs: Vec<Transaction>,
		reward_out: Output,
		reward_kern: TxKernel,
		difficulty: Difficulty,
	) -> Result<Block, Error> {
		// A block is just a big transaction, aggregate and add the reward output
		// and reward kernel. At this point the tx is technically invalid but the
		// tx body is valid if we account for the reward (i.e. as a block).
		let agg_tx = transaction::aggregate(txs)?
			.with_output(reward_out)
			.with_kernel(reward_kern);

		// Now add the kernel offset of the previous block for a total
		let total_kernel_offset = committed::sum_kernel_offsets(
			vec![agg_tx.offset.clone(), prev.total_kernel_offset.clone()],
			vec![],
		)?;

		let total_issue_overage = if let Some(issue_overage) = agg_tx.body.mint_overage()? {
			prev.add_issue_overage(issue_overage)?
		} else {
			prev.total_issue_overage
		};
		// Determine the height and associated version for the new header.

		let height = prev.height + 1;
		let version = consensus::header_version(height);

		let now = Utc::now().timestamp();
		let timestamp = DateTime::<Utc>::from_utc(NaiveDateTime::from_timestamp(now, 0), Utc);

		// Now build the block with all the above information.
		// Note: We have not validated the block here.
		// Caller must validate the block as necessary.
		Block {
			header: BlockHeader {
				version,
				height,
				timestamp,
				prev_hash: prev.hash(),
				total_kernel_offset,
				pow: ProofOfWork {
					total_difficulty: difficulty + prev.pow.total_difficulty,
					..Default::default()
				},
				total_issue_overage,
				..Default::default()
			},
			body: agg_tx.into(),
		}
		.cut_through()
	}

	/// Consumes this block and returns a new block with the coinbase output
	/// and kernels added
	pub fn with_reward(mut self, reward_out: Output, reward_kern: TxKernel) -> Block {
		self.body.outputs = vec![reward_out];
		self.body.kernels = vec![reward_kern];
		self
	}

	/// Get inputs
	pub fn inputs(&self) -> &Vec<Input> {
		&self.body.inputs
	}

	/// Get asset actions
	pub fn assets(&self) -> &Vec<AssetAction> {
		&self.body.assets
	}

	/// Get inputs mutable
	pub fn inputs_mut(&mut self) -> &mut Vec<Input> {
		&mut self.body.inputs
	}

	/// Get outputs
	pub fn outputs(&self) -> &Vec<Output> {
		&self.body.outputs
	}

	/// Get outputs mutable
	pub fn outputs_mut(&mut self) -> &mut Vec<Output> {
		&mut self.body.outputs
	}

	/// Get kernels
	pub fn kernels(&self) -> &Vec<TxKernel> {
		&self.body.kernels
	}

	/// Get kernels mut
	pub fn kernels_mut(&mut self) -> &mut Vec<TxKernel> {
		&mut self.body.kernels
	}

	/// Sum of all fees (inputs less outputs) in the block
	pub fn total_fees(&self) -> u64 {
		self.body.fee()
	}

	/// Get asset issue/create overage
	pub fn mint_overage(&self) -> Result<Option<Commitment>, Error> {
		self.body.mint_overage().map_err(|e| Error::Transaction(e))
	}

	/// Matches any output with a potential spending input, eliminating them
	/// from the block. Provides a simple way to cut-through the block. The
	/// elimination is stable with respect to the order of inputs and outputs.
	/// Method consumes the block.
	pub fn cut_through(self) -> Result<Block, Error> {
		let mut inputs = self.inputs().clone();
		let mut outputs = self.outputs().clone();
		let assets = self.assets().clone();
		transaction::cut_through(&mut inputs, &mut outputs)?;

		let kernels = self.kernels().clone();

		// Initialize tx body and sort everything.
		let body = TransactionBody::init(inputs, outputs, kernels, assets, false)?;

		Ok(Block {
			header: self.header,
			body,
		})
	}

	/// "Lightweight" validation that we can perform quickly during read/deserialization.
	/// Subset of full validation that skips expensive verification steps, specifically -
	/// * rangeproof verification (on the body)
	/// * kernel signature verification (on the body)
	/// * coinbase sum verification
	/// * kernel sum verification
	pub fn validate_read(&self) -> Result<(), Error> {
		self.body.validate_read(Weighting::AsBlock)?;
		self.verify_kernel_lock_heights()?;
		Ok(())
	}

	fn block_kernel_offset(
		&self,
		prev_kernel_offset: BlindingFactor,
	) -> Result<BlindingFactor, Error> {
		let offset = if self.header.total_kernel_offset() == prev_kernel_offset {
			// special case when the sum hasn't changed (typically an empty block),
			// zero isn't a valid private key but it's a valid blinding factor
			BlindingFactor::zero()
		} else {
			committed::sum_kernel_offsets(
				vec![self.header.total_kernel_offset()],
				vec![prev_kernel_offset],
			)?
		};
		Ok(offset)
	}

	/// Validates all the elements in a block that can be checked without
	/// additional data. Includes commitment sums and kernels, Merkle
	/// trees, reward, etc.
	pub fn validate(
		&self,
		prev_kernel_offset: &BlindingFactor,
		verifier: Arc<RwLock<dyn VerifierCache>>,
	) -> Result<Commitment, Error> {
		self.body.validate(Weighting::AsBlock, verifier)?;

		self.verify_kernel_lock_heights()?;
		self.verify_coinbase()?;

		// mint asset amount
		// let sum = self.assets().iter().fold(0u128, |sum, a| sum + a.amount());
		let mint_overage = self.mint_overage()?;

		// take the kernel offset for this block (block offset minus previous) and
		// verify.body.outputs and kernel sums
		// TODO add mint amount to it
		let (_utxo_sum, kernel_sum) = self.verify_kernel_sums(
			self.header.overage(),
			mint_overage,
			self.block_kernel_offset(prev_kernel_offset.clone())?,
		)?;

		Ok(kernel_sum)
	}

	/// Validate the coinbase.body.outputs generated by miners.
	/// Check the sum of coinbase-marked outputs match
	/// the sum of coinbase-marked kernels accounting for fees.
	pub fn verify_coinbase(&self) -> Result<(), Error> {
		let cb_outs = self
			.body
			.outputs
			.iter()
			.filter(|out| out.is_coinbase())
			.collect::<Vec<&Output>>();

		let cb_kerns = self
			.body
			.kernels
			.iter()
			.filter(|kernel| kernel.is_coinbase())
			.collect::<Vec<&TxKernel>>();

		{
			let secp = static_secp_instance();
			let secp = secp.lock();

			let over_commit = secp.commit_value(reward(self.header.height, self.total_fees()))?;

			let out_adjust_sum =
				secp.commit_sum(map_vec!(cb_outs, |x| x.commitment()), vec![over_commit])?;

			let kerns_sum = secp.commit_sum(cb_kerns.iter().map(|x| x.excess).collect(), vec![])?;

			// Verify the kernel sum equals the output sum accounting for block fees.
			if kerns_sum != out_adjust_sum {
				return Err(Error::CoinbaseSumMismatch);
			}
		}

		Ok(())
	}

	fn verify_kernel_lock_heights(&self) -> Result<(), Error> {
		for k in &self.body.kernels {
			// check we have no kernels with lock_heights greater than current height
			// no tx can be included in a block earlier than its lock_height
			if let KernelFeatures::HeightLocked { lock_height, .. } = k.features {
				if lock_height > self.header.height {
					return Err(Error::KernelLockHeight(lock_height));
				}
			}
		}
		Ok(())
	}
}

impl From<UntrustedBlock> for Block {
	fn from(block: UntrustedBlock) -> Self {
		block.0
	}
}

/// Block which does lightweight validation as part of deserialization,
/// it supposed to be used when we can't trust the channel (eg network)
pub struct UntrustedBlock(Block);

/// Deserialization of an untrusted block header
impl Readable for UntrustedBlock {
	fn read(reader: &mut dyn Reader) -> Result<UntrustedBlock, ser::Error> {
		// we validate header here before parsing the body
		let header = UntrustedBlockHeader::read(reader)?;
		let body = TransactionBody::read(reader)?;

		// Now "lightweight" validation of the block.
		// Treat any validation issues as data corruption.
		// An example of this would be reading a block
		// that exceeded the allowed number of inputs.
		body.validate_read(Weighting::AsBlock).map_err(|e| {
			error!("read validation error: {}", e);
			ser::Error::CorruptedData
		})?;
		let block = Block {
			header: header.into(),
			body,
		};
		Ok(UntrustedBlock(block))
	}
}
