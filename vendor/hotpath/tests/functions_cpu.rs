#[cfg(test)]
pub mod tests {
    use std::process::Command;

    // cargo run -p test-tokio-async --example cpu_basic --features hotpath,hotpath-cpu
    #[test]
    fn test_cpu_basic_output() {
        let output = Command::new("cargo")
            .args([
                "run",
                "-p",
                "test-tokio-async",
                "--example",
                "cpu_basic",
                "--features",
                "hotpath,hotpath-cpu",
            ])
            .env("HOTPATH_REPORT", "functions-cpu")
            .output()
            .expect("Failed to execute command");

        assert!(
            output.status.success(),
            "Process did not exit successfully.\n\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        let expected_content = ["cpu_basic::heavy_work", "cpu_basic::light_work"];

        for expected in expected_content {
            assert!(
                stdout.contains(expected),
                "Expected:\n{expected}\n\nGot:\n{stdout}",
            );
        }
    }

    // HOTPATH_OUTPUT_FORMAT=json HOTPATH_REPORT=functions-cpu cargo run -p test-tokio-async --example cpu_symbols --features hotpath,hotpath-cpu
    #[test]
    fn test_cpu_symbols_output() {
        let output = Command::new("cargo")
            .args([
                "run",
                "-p",
                "test-tokio-async",
                "--example",
                "cpu_symbols",
                "--features",
                "hotpath,hotpath-cpu",
            ])
            .env("HOTPATH_OUTPUT_FORMAT", "json")
            .env("HOTPATH_REPORT", "functions-cpu")
            .output()
            .expect("Failed to execute command");

        assert!(
            output.status.success(),
            "Process did not exit successfully.\n\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json_line = stdout.lines().last().expect("no output");

        let expected_symbols = [
            "cpu_symbols::free_heavy_work",
            "Worker::method_heavy_work",
            "Worker::method_light_work",
            "OtherWorker::method_heavy_work",
        ];

        for expected in expected_symbols {
            assert!(
                json_line.contains(expected),
                "Expected symbol:\n{expected}\n\nGot:\n{json_line}",
            );
        }
    }

    // cargo run -p test-tokio-async --example cpu_inline --features hotpath,hotpath-cpu
    #[test]
    fn test_cpu_inline_default_strips_user_inline() {
        let output = Command::new("cargo")
            .args([
                "run",
                "-p",
                "test-tokio-async",
                "--example",
                "cpu_inline",
                "--features",
                "hotpath,hotpath-cpu",
                "--release",
            ])
            .env("HOTPATH_REPORT", "functions-cpu")
            .env("CARGO_TARGET_DIR", "target/test-cpu-inline-default")
            .output()
            .expect("Failed to execute command");

        assert!(
            output.status.success(),
            "Process did not exit successfully.\n\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);

        for expected in ["cpu_inline::never_inlined", "cpu_inline::always_inlined"] {
            assert!(
                stdout.contains(expected),
                "Expected:\n{expected}\n\nGot:\n{stdout}",
            );
        }
    }

    // HOTPATH_KEEP_INLINE=1 cargo run -p test-tokio-async --example cpu_inline --features hotpath,hotpath-cpu
    #[test]
    fn test_cpu_inline_keep_inline_preserves_user_inline() {
        let output = Command::new("cargo")
            .args([
                "run",
                "-p",
                "test-tokio-async",
                "--example",
                "cpu_inline",
                "--features",
                "hotpath,hotpath-cpu",
                "--release",
            ])
            .env("HOTPATH_REPORT", "functions-cpu")
            .env("HOTPATH_KEEP_INLINE", "1")
            .env("CARGO_TARGET_DIR", "target/test-cpu-inline-keep")
            .output()
            .expect("Failed to execute command");

        assert!(
            output.status.success(),
            "Process did not exit successfully.\n\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(
            stdout.contains("cpu_inline::never_inlined"),
            "Expected:\ncpu_inline::never_inlined\n\nGot:\n{stdout}",
        );
        assert!(
            !stdout.contains("cpu_inline::always_inlined"),
            "Expected always_inlined to be missing under HOTPATH_KEEP_INLINE=1.\n\nGot:\n{stdout}",
        );
    }

    // cargo run -p test-tokio-async --example cpu_labels --features hotpath,hotpath-cpu
    #[test]
    fn test_cpu_labels_output() {
        let output = Command::new("cargo")
            .args([
                "run",
                "-p",
                "test-tokio-async",
                "--example",
                "cpu_labels",
                "--features",
                "hotpath,hotpath-cpu",
            ])
            .env("HOTPATH_REPORT", "functions-cpu")
            .output()
            .expect("Failed to execute command");

        assert!(
            output.status.success(),
            "Process did not exit successfully.\n\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        let expected_content = ["custom_heavy", "cpu_labels::heavy_no_label"];

        for expected in expected_content {
            assert!(
                stdout.contains(expected),
                "Expected:\n{expected}\n\nGot:\n{stdout}",
            );
        }
    }
}
