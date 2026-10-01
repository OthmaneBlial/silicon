use std::{fs, process::Command};

#[test]
fn four_sample_capture_replays_the_rendered_image() {
    let directory =
        std::env::temp_dir().join(format!("silicon-cli-capture-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let capture = directory.join("frame.silicon");
    let rendered = directory.join("rendered.png");
    let replayed = directory.join("replayed.png");
    let cli = env!("CARGO_BIN_EXE_silicon");

    let output = Command::new(cli)
        .args([
            "render",
            "shader_cube",
            "--width",
            "64",
            "--height",
            "48",
            "--samples",
            "4",
            "--capture",
        ])
        .arg(&capture)
        .arg("--output")
        .arg(&rendered)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let output = Command::new(cli)
        .arg("inspect")
        .arg(&capture)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("4 samples/pixel"));

    let output = Command::new(cli)
        .arg("replay")
        .arg(&capture)
        .arg("--output")
        .arg(&replayed)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(rendered).unwrap(), fs::read(replayed).unwrap());
    fs::remove_dir_all(directory).unwrap();
}
