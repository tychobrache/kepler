// Copyright 2019 The Kepler Developers
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

//! Transaction integration tests

pub mod common;

use self::core::core::{Output, OutputFeatures};
use self::core::libtx::proof;
use self::core::ser;
use self::keychain::{ExtKeychain, Keychain};
use kepler_core as core;
use kepler_keychain as keychain;

#[test]
fn test_output_ser_deser() {
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let switch = &keychain::SwitchCommitmentType::Regular;
	let commit = keychain.commit(5, &key_id, switch).unwrap();
	let builder = proof::ProofBuilder::new(&keychain);
	let proof = proof::create(&keychain, &builder, 5, &key_id, switch, commit, None).unwrap();

	let out = Output {
		features: OutputFeatures::Plain,
		commit: commit,
		proof: proof,
	};

	let mut vec = vec![];
	ser::serialize_default(&mut vec, &out).expect("serialized failed");
	let dout: Output = ser::deserialize_default(&mut &vec[..]).unwrap();

	assert_eq!(dout.features, OutputFeatures::Plain);
	assert_eq!(dout.commit, out.commit);
	assert_eq!(dout.proof, out.proof);
}
