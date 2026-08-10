/// Thread count tuned for Apple Silicon (P-cores first).
pub fn export_jobs(requested: Option<usize>) -> usize {
	if let Some(n) = requested {
		return n.max(1);
	}
	if let Ok(s) = std::env::var("LIGHT_JOBS") {
		if let Ok(n) = s.parse::<usize>() {
			return n.max(1);
		}
	}
	performance_core_count().unwrap_or_else(num_cpus)
}

fn num_cpus() -> usize {
	std::thread::available_parallelism()
		.map(|n| n.get())
		.unwrap_or(4)
		.max(1)
}

#[cfg(target_os = "macos")]
fn performance_core_count() -> Option<usize> {
	let out = std::process::Command::new("sysctl")
		.args(["-n", "hw.perflevel0.physicalcpu"])
		.output()
		.ok()?;
	if !out.status.success() {
		return None;
	}
	let n: usize = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
	Some(n.max(1))
}

#[cfg(not(target_os = "macos"))]
fn performance_core_count() -> Option<usize> {
	None
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn export_jobs_honors_requested_count() {
		assert_eq!(export_jobs(Some(8)), 8);
		assert_eq!(export_jobs(Some(0)), 1);
	}

	#[test]
	fn export_jobs_defaults_to_at_least_one() {
		assert!(export_jobs(None) >= 1);
	}
}
