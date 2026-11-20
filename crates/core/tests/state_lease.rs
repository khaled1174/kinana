//! A child process started on one thread must not make the writer lease
//! look busy to another thread of the same process (false `STATE_BUSY`).
use kinana_core::state;
use std::fs;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(unix)]
#[test]
fn spawning_children_never_makes_a_released_lease_look_busy() {
    let home = fs::canonicalize(std::env::temp_dir())
        .unwrap()
        .join(format!(
            "kinana-lease-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
    fs::create_dir_all(home.join(".kinana")).unwrap();
    let index = home.join(".kinana").join("index-v2.db");
    let stop = Arc::new(AtomicBool::new(false));
    let spawner = {
        let stop = stop.clone();
        std::thread::spawn(move || {
            let mut children = 0;
            while !stop.load(Ordering::Relaxed) {
                let _ = Command::new("/usr/bin/true")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
                children += 1;
            }
            children
        })
    };
    let mut busy = 0;
    let tries = 20_000;
    for _ in 0..tries {
        match state::write_guard(&index, false) {
            Ok(guard) => drop(guard),
            Err(error) if error.code == "STATE_BUSY" => busy += 1,
            Err(error) => panic!("{}: {}", error.code, error.message),
        }
    }
    stop.store(true, Ordering::Relaxed);
    let children = spawner.join().unwrap();
    let _ = fs::remove_dir_all(&home);
    assert!(children > 0, "no child process ran during the test");
    assert_eq!(
        busy, 0,
        "false STATE_BUSY in {busy} of {tries} acquisitions"
    );
}

/// Third review, finding 6: guards taken and dropped on several threads at
/// once. Inside one process a lease is shared, so any `STATE_BUSY` here is
/// false: the last guard must unlock before another thread can try the lock.
#[cfg(unix)]
#[test]
fn guards_dropped_on_other_threads_never_make_the_lease_look_busy() {
    let home = fs::canonicalize(std::env::temp_dir())
        .unwrap()
        .join(format!(
            "kinana-lease-threads-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
    fs::create_dir_all(home.join(".kinana")).unwrap();
    let index = Arc::new(home.join(".kinana").join("index-v2.db"));
    let workers = (0..8)
        .map(|_| {
            let index = index.clone();
            std::thread::spawn(move || {
                let mut busy = 0;
                for _ in 0..5_000 {
                    match state::write_guard(&index, false) {
                        Ok(guard) => drop(guard),
                        Err(error) if error.code == "STATE_BUSY" => busy += 1,
                        Err(error) => panic!("{}: {}", error.code, error.message),
                    }
                }
                busy
            })
        })
        .collect::<Vec<_>>();
    let busy: usize = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .sum();
    let _ = fs::remove_dir_all(&home);
    assert_eq!(busy, 0, "false STATE_BUSY in {busy} of 40,000 acquisitions");
}

fn fresh_home(label: &str) -> std::path::PathBuf {
    let home = fs::canonicalize(std::env::temp_dir())
        .unwrap()
        .join(format!(
            "kinana-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
    fs::create_dir_all(&home).unwrap();
    home
}

/// Child side of the cross-process race below: create the lease file the way
/// `init` does, then take the lease as every writer does.
#[test]
fn lease_race_child() {
    let Some(home) = std::env::var_os("KINANA_LEASE_RACE_HOME") else {
        return;
    };
    let home = std::path::PathBuf::from(home);
    let mode = std::env::var("KINANA_LEASE_RACE_MODE").unwrap();
    if mode == "ensure" {
        state::ensure_lease_file(&home).unwrap();
    }
    match state::write_guard(&home.join(".kinana/index-v2.db"), false) {
        Ok(_guard) => {}
        // Another process holding the lease right now is a true answer.
        Err(error) if error.code == "STATE_BUSY" => {}
        Err(error) => panic!("{}: {}", error.code, error.message),
    }
}

#[cfg(unix)]
#[test]
fn processes_racing_on_a_fresh_home_never_see_a_half_written_lease_file() {
    let exe = std::env::current_exe().unwrap();
    for round in 0..40 {
        let home = fresh_home(&format!("lease-race-{round}"));
        let children: Vec<_> = (0..12)
            .map(|_| {
                Command::new(&exe)
                    .args([
                        "--exact",
                        "lease_race_child",
                        "--nocapture",
                        "--test-threads=1",
                    ])
                    .env("KINANA_LEASE_RACE_HOME", &home)
                    .env("KINANA_LEASE_RACE_MODE", "ensure")
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap()
            })
            .collect();
        for child in children {
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "round {round}: {}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
        assert_eq!(
            fs::read(home.join(".kinana-write.lock")).unwrap(),
            b"Kinana state writer lease v1\n"
        );
        // No staging file is left behind.
        let left: Vec<_> = fs::read_dir(home.join(".kinana"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert!(left.is_empty(), "{left:?}");
        fs::remove_dir_all(&home).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn the_lease_file_is_created_private_and_existing_ones_are_never_rewritten() {
    use std::os::unix::fs::PermissionsExt;
    let home = fresh_home("lease-file");
    state::ensure_lease_file(&home).unwrap();
    let lock = home.join(".kinana-write.lock");
    assert_eq!(fs::read(&lock).unwrap(), b"Kinana state writer lease v1\n");
    assert_eq!(
        fs::metadata(&lock).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let before = fs::metadata(&lock).unwrap();
    state::ensure_lease_file(&home).unwrap();
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(fs::metadata(&lock).unwrap().ino(), before.ino());
    }
    // A foreign file in its place is reported and kept.
    fs::write(&lock, "not ours").unwrap();
    assert_eq!(
        state::ensure_lease_file(&home).unwrap_err().code,
        "STATE_CONFLICT"
    );
    assert_eq!(fs::read(&lock).unwrap(), b"not ours");
    fs::remove_dir_all(&home).unwrap();

    // Legacy state is read-only until `migrate`: nothing is created.
    let legacy = fresh_home("lease-legacy");
    fs::create_dir(legacy.join(".skillsbox")).unwrap();
    state::ensure_lease_file(&legacy).unwrap();
    assert!(!legacy.join(".kinana-write.lock").exists());
    assert!(!legacy.join(".kinana").exists());
    fs::remove_dir_all(&legacy).unwrap();
}
