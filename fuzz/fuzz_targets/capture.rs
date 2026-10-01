#![no_main]

use libfuzzer_sys::fuzz_target;
use silicon_core::FrameCapture;

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > 64 * 1024 * 1024 {
        return;
    }
    let Ok(capture) = serde_json::from_slice::<FrameCapture>(bytes) else {
        return;
    };
    let _ = capture.commands.validate();
    if capture.width <= 64 && capture.height <= 64 {
        let _ = capture.replay();
    }
});
