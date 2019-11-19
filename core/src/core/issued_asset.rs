//use serde::de::{Deserialize, Deserializer, SeqAccess, Visitor};
//use serde::ser::{Serialize, SerializeSeq, Serializer};
use serde_derive::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

use crate::core::hash::DefaultHashable;
use crate::ser::{self, FixedLength, PMMRable, Readable, Reader, Writeable, Writer};
use crate::util::secp::{key::PublicKey, ContextFlag, Message, Secp256k1, Signature};

use super::asset::Asset;

#[derive(Copy, Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
pub enum AssetAction {
	New(Asset, IssuedAsset, Signature),
	Issue(Asset, u128, Signature),
	Withdraw(Asset, u128, Signature),
	ChangeOwner(Asset, PublicKey, Signature),
}

impl AssetAction {
	pub fn valid(&self, pk: &PublicKey) -> bool {
		let (bytes, sign) = match self {
			AssetAction::New(_, issue, sign) => (bincode::serialize(&issue).unwrap(), sign),
			AssetAction::Issue(_, num, sign) => (bincode::serialize(&num).unwrap(), sign),
			AssetAction::Withdraw(_, num, sign) => (bincode::serialize(&num).unwrap(), sign),
			AssetAction::ChangeOwner(_, pk, sign) => (bincode::serialize(&pk).unwrap(), sign),
		};
		let message = &Message::from_bytes(&bytes).unwrap();
		let secp = Secp256k1::with_caps(ContextFlag::VerifyOnly);
		secp.verify(&message, &sign, pk).is_ok()
	}

	pub fn asset(&self) -> Asset {
		match self {
			AssetAction::New(asset, _, _)
			| AssetAction::Issue(asset, _, _)
			| AssetAction::Withdraw(asset, _, _)
			| AssetAction::ChangeOwner(asset, _, _) => asset.clone(),
		}
	}

	pub fn amount(&self) -> u128 {
		match self {
			AssetAction::New(_, asset, _) => *asset.supply(),
			AssetAction::Issue(_, amount, _) | AssetAction::Withdraw(_, amount, _) => *amount,
			AssetAction::ChangeOwner(asset, _, _) => 0u128,
		}
	}

	pub fn is_new(&self) -> bool {
		match self {
			AssetAction::New(_, _, _) => true,
			_ => false,
		}
	}

	pub fn issued_asset(&self) -> Option<IssuedAsset> {
		match self {
			AssetAction::New(_, issued_asset, _) => Some(issued_asset.clone()),
			_ => None,
		}
	}
}

#[derive(Copy, Clone, Serialize, Deserialize, Debug, Eq, PartialEq)]
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

	pub fn new(supply: u128, owner: PublicKey, mintable: bool, asset: Asset) -> Self {
		Self {
			supply,
			owner,
			mintable,
			asset,
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

impl Readable for AssetAction {
	fn read(reader: &mut dyn Reader) -> Result<AssetAction, ser::Error> {
		let len = reader.read_u32()?;
		let vec = reader.read_fixed_bytes(len as usize)?;

		bincode::deserialize::<AssetAction>(&vec[..]).map_err(|_| {
			ser::Error::IOErr(
				"asset action deserialize error".to_owned(),
				std::io::ErrorKind::InvalidInput,
			)
		})
	}
}

impl Writeable for AssetAction {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		let vec = bincode::serialize(&self).map_err(|_| {
			ser::Error::IOErr(
				"asset action deserialize error".to_owned(),
				std::io::ErrorKind::InvalidInput,
			)
		})?;
		let len = vec.len();
		writer.write_u32(len as u32);
		writer.write_fixed_bytes(&vec)?;

		Ok(())
	}
}

impl Hash for AssetAction {
	fn hash<H: Hasher>(&self, state: &mut H) {
		bincode::serialize(&self).unwrap().hash(state);
	}
}

// impl FixedLength for AssetAction {
// 	const LEN: usize = 114;
// }

// impl PMMRable for AssetAction {
// 	type E = Self;

// 	fn as_elmt(&self) -> Self::E {
// 		self.clone()
// 	}
// }

// impl DefaultHashable for AssetAction {}
