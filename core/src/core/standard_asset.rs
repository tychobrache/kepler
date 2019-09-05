use crate::util::secp::{key::PublicKey, ContextFlag, Message, Secp256k1, Signature};

use super::asset::Asset;

pub enum AssetTotalSupply {
	Mutable(u128),
	Immutable(u128),
}

pub enum AssetOwner {
	Coinbase,
	Owner(PublicKey),
}

pub struct StandardAsset {
	total_supply: AssetTotalSupply,
	owner: AssetOwner,
	symbol: String,
	name: String,
}

impl StandardAsset {
	pub fn total_supply(&self) -> &u128 {
		match self.total_supply {
			AssetTotalSupply::Mutable(ref n) => n,
			AssetTotalSupply::Immutable(ref n) => n,
		}
	}

	pub fn owner(&self) -> &AssetOwner {
		&self.owner
	}

	pub fn symbol(&self) -> &String {
		&self.symbol
	}

	pub fn name(&self) -> &String {
		&self.name
	}

	pub fn new(
		total_supply: AssetTotalSupply,
		owner: AssetOwner,
		symbol: String,
		name: String,
	) -> Self {
		Self {
			total_supply,
			owner,
			symbol,
			name,
		}
	}

	pub fn change_owner_message(&self, new_pk: PublicKey) -> Message {
		// TODO secp message
		[0; 32].into()
	}

	pub fn change_owner(&mut self, new_pk: PublicKey, sign: Signature) -> bool {
		let message = &self.change_owner_message(new_pk);

		match self.owner {
			AssetOwner::Coinbase => false,
			AssetOwner::Owner(ref mut pk) => {
				let secp = Secp256k1::with_caps(ContextFlag::VerifyOnly);
				if secp.verify(&message, &sign, pk).is_ok() {
					*pk = new_pk;
					true
				} else {
					false
				}
			}
		}
	}

	pub fn to_asset(&self) -> Asset {
		(&self.symbol[..]).into()
	}
}
