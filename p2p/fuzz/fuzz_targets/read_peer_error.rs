#![no_main]
#[macro_use]
extern crate libfuzzer_sys;
extern crate kepler_core;
extern crate kepler_p2p;

use kepler_core::ser;
use kepler_p2p::msg::PeerError;

fuzz_target!(|data: &[u8]| {
	let mut d = data.clone();
	let _t: Result<PeerError, ser::Error> = ser::deserialize(&mut d);
});
