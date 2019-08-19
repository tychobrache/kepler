use serde::de::{Deserialize, Deserializer, SeqAccess, Visitor};
use serde::ser::{Serialize, SerializeSeq, Serializer};

use std::fmt::{Debug, Display, Formatter, Result as FmtResult};

use crate::util::secp::ffi::Generator;

const MAIN_ASSET: [u8; 64] = [0u8; 64];

#[derive(Copy, Clone)]
pub struct Asset([u8; 64]);

impl Asset {
	pub fn from_generator(g: Generator) -> Self {
		Asset::from_bytes(g.0)
	}

	pub fn from_bytes(bytes: [u8; 64]) -> Self {
		Asset(bytes)
	}
}

impl Default for Asset {
	fn default() -> Asset {
		Asset::from_bytes(MAIN_ASSET)
	}
}

impl From<Asset> for Generator {
	fn from(asset: Asset) -> Generator {
		Generator(asset.0)
	}
}

impl<'a> From<&'a Asset> for Generator {
	fn from(asset: &Asset) -> Generator {
		Generator(asset.0)
	}
}

impl Debug for Asset {
	fn fmt(&self, f: &mut Formatter) -> FmtResult {
		let mut hex = String::new();
		hex.extend(self.0.iter().map(|byte| format!("{:02x?}", byte)));
		write!(f, "Asset: 0x{}", hex)
	}
}

impl Serialize for Asset {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
		for e in self.0.iter() {
			seq.serialize_element(e)?;
		}
		seq.end()
	}
}

impl<'d> Deserialize<'d> for Asset {
	fn deserialize<D>(deserializer: D) -> Result<Asset, D::Error>
	where
		D: Deserializer<'d>,
	{
		struct AssetVistor;

		impl<'de> Visitor<'de> for AssetVistor {
			type Value = Asset;

			fn expecting(&self, formatter: &mut Formatter) -> FmtResult {
				formatter.write_str(concat!("an array of length ", 64))
			}

			fn visit_seq<A>(self, mut seq: A) -> Result<Asset, A::Error>
			where
				A: SeqAccess<'de>,
			{
				let mut arr = [0u8; 64];
				for i in 0..64 {
					arr[i] = seq
						.next_element()?
						.ok_or_else(|| serde::de::Error::invalid_length(i, &self))?;
				}
				Ok(Asset::from_bytes(arr))
			}
		}

		deserializer.deserialize_seq(AssetVistor)
	}
}
