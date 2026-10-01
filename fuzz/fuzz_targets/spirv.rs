#![no_main]

use libfuzzer_sys::fuzz_target;
use silicon_shader::spirv::Module;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(module) = Module::parse(bytes) {
        let _ = module.translate();
    }
});
