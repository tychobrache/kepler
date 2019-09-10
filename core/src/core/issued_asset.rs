//use serde::de::{Deserialize, Deserializer, SeqAccess, Visitor};
//use serde::ser::{Serialize, SerializeSeq, Serializer};
use serde_derive::{Deserialize, Serialize};

use crate::core::hash::DefaultHashable;
use crate::ser::{self, FixedLength, PMMRable, Readable, Reader, Writeable, Writer};
use crate::util::secp::{key::PublicKey, ContextFlag, Message, Secp256k1, Signature};

use super::asset::Asset;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct IssuedAsset {
	supply: u128,
	owner: PublicKey,
	mintable: bool,
	asset: Asset,
}

impl IssuedAsset {
	pub fn supply(&self) -> &u128 {
		&self.supply
	}

	pub fn owner(&self) -> &PublicKey {
		&self.owner
	}

	pub fn mintable(&self) -> bool {
		self.mintable
	}

	pub fn asset(&self) -> &Asset {
		&self.asset
	}

	pub fn new(supply: u128, owner: PublicKey, mintable: bool, seed: &str) -> Self {
		Self {
			supply: supply,
			owner: owner,
			mintable: mintable,
			asset: seed.into(),
		}
	}

	pub fn change_owner_message(&self, new_pk: PublicKey) -> Message {
		// TODO secp message
		[0; 32].into()
	}

	pub fn change_owner(&mut self, new_pk: PublicKey, sign: Signature) -> bool {
		let message = &self.change_owner_message(new_pk);
		let secp = Secp256k1::with_caps(ContextFlag::VerifyOnly);
		if secp.verify(&message, &sign, &self.owner).is_ok() {
			self.owner = new_pk;
			return true;
		}

		false
	}
}

impl Readable for IssuedAsset {
	fn read(reader: &mut dyn Reader) -> Result<IssuedAsset, ser::Error> {
		let vec = reader.read_fixed_bytes(114)?;

		// supply: u128, 16 bytes
		let mut supply_bytes = [0u8; 16];
		supply_bytes.copy_from_slice(&vec[0..16]);
		let supply = u128::from_be_bytes(supply_bytes);

		// owner: PublicKey,  compress 33 bytes serialize_vec(
		let secp = Secp256k1::with_caps(ContextFlag::None);
		let owner = PublicKey::from_slice(&secp, &vec[16..49]).map_err(|_| {
			ser::Error::IOErr(
				"public key deserialize error".to_owned(),
				std::io::ErrorKind::InvalidInput,
			)
		})?;

		// mintable: bool, 1 bytes
		let mintable = vec[49] == 1u8;

		// asset: Asset, 64 bytes
		let mut asset_bytes = [0u8; 64];
		asset_bytes.copy_from_slice(&vec[50..114]);
		let asset = Asset::from_bytes(asset_bytes);

		Ok(IssuedAsset {
			supply,
			owner,
			mintable,
			asset,
		})
	}
}

impl Writeable for IssuedAsset {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		let supply_bytes = self.supply.to_le_bytes();
		writer.write_fixed_bytes(&(&supply_bytes[..]))?;

		let secp = Secp256k1::with_caps(ContextFlag::None);
		let public_key_bytes = self.owner.serialize_vec(&secp, true);
		writer.write_fixed_bytes(&(&public_key_bytes[..]))?;

		let mintable_bytes = if self.mintable { [1u8] } else { [0u8] };
		writer.write_fixed_bytes(&mintable_bytes)?;

		writer.write_fixed_bytes(&self.asset.to_bytes())?;

		Ok(())
	}
}

impl FixedLength for IssuedAsset {
	const LEN: usize = 114;
}

impl PMMRable for IssuedAsset {
	type E = Self;

	fn as_elmt(&self) -> Self::E {
		self.clone()
	}
}

impl DefaultHashable for IssuedAsset {}