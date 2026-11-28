use std::{env, process::Command};

const SUBPROCESS_ENV: &str = "NUMEN_LOG_TEST_SUBPROCESS";

fn capture_stderr(test_name: &str, child_body: impl FnOnce()) -> Option<String> {
	if env::var_os(SUBPROCESS_ENV).is_some() {
		child_body();
		return None;
	}

	let output = Command::new(env::current_exe().expect("test binary path"))
		.env(SUBPROCESS_ENV, "1")
		.args(["--exact", test_name, "--nocapture"])
		.output()
		.expect("spawn logging test subprocess");

	assert!(
		output.status.success(),
		"subprocess failed with status {:?}, stderr: {}",
		output.status,
		String::from_utf8_lossy(&output.stderr),
	);

	Some(String::from_utf8(output.stderr).expect("utf8 stderr"))
}

#[test]
fn colored_log_messages_keep_raw_ansi_sequences() {
	let Some(stderr) = capture_stderr("colored_log_messages_keep_raw_ansi_sequences", || {
		let mut logger = sc_cli::LoggerBuilder::new("");
		logger.with_colors(true);
		logger.init().expect("logger initializes");
		log::info!("💤 \x1b[37m\x1b[1mIdle\x1b[0m (\x1b[37m\x1b[1m0\x1b[0m peers)");
	}) else {
		return;
	};

	assert!(stderr.starts_with("\x1b[2m"), "expected dimmed timestamp prefix, got {stderr:?}");
	assert!(
		stderr.contains("💤 \x1b[37m\x1b[1mIdle\x1b[0m (\x1b[37m\x1b[1m0\x1b[0m peers)\n"),
		"expected raw ANSI color sequences in message, got {stderr:?}",
	);
	assert!(!stderr.contains("\\x1b"), "ANSI escape byte was escaped as literal text: {stderr:?}");
}

#[test]
fn disabled_log_colors_omit_ansi_codes() {
	let Some(stderr) = capture_stderr("disabled_log_colors_omit_ansi_codes", || {
		let mut logger = sc_cli::LoggerBuilder::new("");
		logger.with_colors(false);
		logger.init().expect("logger initializes");
		log::info!("💤 Idle (0 peers)");
	}) else {
		return;
	};

	assert!(!stderr.contains('\x1b'), "unexpected raw ANSI escape in uncolored log: {stderr:?}");
	assert!(!stderr.contains("\\x1b"), "unexpected escaped ANSI text in uncolored log: {stderr:?}");
	assert!(stderr.ends_with(" 💤 Idle (0 peers)\n"), "unexpected log line: {stderr:?}");
}

#[test]
fn detailed_logs_preserve_ansi_level_and_message_codes() {
	let Some(stderr) = capture_stderr("detailed_logs_preserve_ansi_level_and_message_codes", || {
		let mut logger = sc_cli::LoggerBuilder::new("");
		logger.with_colors(true).with_detailed_output(true);
		logger.init().expect("logger initializes");
		log::info!(target: "substrate", "🏆 Imported #\x1b[37m\x1b[1m1\x1b[0m");
	}) else {
		return;
	};

	assert!(stderr.contains("\x1b[32m INFO\x1b[0m"), "expected green INFO level, got {stderr:?}");
	assert!(stderr.contains("substrate: 🏆 Imported #\x1b[37m\x1b[1m1\x1b[0m\n"), "got {stderr:?}");
	assert!(!stderr.contains("\\x1b"), "ANSI escape byte was escaped as literal text: {stderr:?}");
}
