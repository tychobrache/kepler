use serde::de::{Deserialize, Deserializer, SeqAccess, Visitor};
use serde::ser::{Serialize, SerializeSeq, Serializer};
use std::cmp::Ordering;
use std::convert::AsRef;
use std::fmt::{Debug, Formatter, Result as FmtResult};
use std::hash::{Hash, Hasher};

use crate::core::hash::DefaultHashable;
use crate::ser::{self, FixedLength, PMMRable, Readable, Reader, Writeable, Writer};
use crate::util::secp::constants::GENERATOR_H;
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
		Asset(GENERATOR_H)
		//Asset::from_bytes(MAIN_ASSET)
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

impl Readable for Asset {
	fn read(reader: &mut dyn Reader) -> Result<Asset, ser::Error> {
		let vec = reader.read_fixed_bytes(64)?;
		let mut bytes = [0u8; 64];
		bytes.copy_from_slice(&vec[..]);

		Ok(Asset::from_bytes(bytes))
	}
}

impl Writeable for Asset {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		let bytes: Vec<u8> = self.0.to_vec();
		writer.write_fixed_bytes(&bytes)?;
		Ok(())
	}
}

impl Eq for Asset {}

impl Ord for Asset {
	fn cmp(&self, other: &Asset) -> Ordering {
		self.0.cmp(&other.0)
	}
}

impl PartialOrd for Asset {
	fn partial_cmp(&self, other: &Asset) -> Option<Ordering> {
		Some(self.cmp(other))
	}
}

impl PartialEq for Asset {
	fn eq(&self, other: &Asset) -> bool {
		self.0[..] == other.0[..]
	}
}

impl Hash for Asset {
	fn hash<H: Hasher>(&self, state: &mut H) {
		let mut hex = String::new();
		hex.extend(self.0.iter().map(|byte| format!("{:02x?}", byte)));
		hex.hash(state);
	}
}

impl AsRef<[u8]> for Asset {
	fn as_ref(&self) -> &[u8] {
		&self.0
	}
}

impl FixedLength for Asset {
	const LEN: usize = 64;
}

impl PMMRable for Asset {
	type E = Self;

	fn as_elmt(&self) -> Self::E {
		self.clone()
	}
}

impl DefaultHashable for Asset {}
