use super::*;
use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};
pub(crate) fn fixture(script: &str) -> (tempfile::TempDir, PathBuf, Snapshot) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let worker = root.join("fake-worker");
    std::fs::write(&worker, format!("#!/usr/bin/python3\n{script}")).unwrap();
    std::fs::set_permissions(&worker, std::fs::Permissions::from_mode(0o700)).unwrap();
    let proof = Fingerprint::capture(&worker, &|| false).unwrap();
    let snapshot = Snapshot {
        gpu: None,
        protocol: 1,
        engine: "whisper".into(),
        name: "mon-whisper-worker".into(),
        bytes: proof.size,
        sha256: proof.sha256,
    };
    let directory = root.join("inference-runtimes").join(&snapshot.sha256);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::copy(worker, directory.join(&snapshot.name)).unwrap();
    (tmp, root, snapshot)
}
fn start(root: &Path, index: usize) -> Run {
    let dir = root.join(format!("unit-{index}"));
    std::fs::create_dir(&dir).unwrap();
    let mut command = OwnedCommand::new(Path::new("/unused-frozen-cli")).unwrap();
    command.command.arg("--model").arg("/same-model");
    Run::spawn(&mut command, "whisper", &dir, "stdout.log", "stderr.log").unwrap()
}
fn wait(run: &mut Run) -> Result<(), String> {
    let start = Instant::now();
    loop {
        if run.try_wait()?.is_some() {
            return Ok(());
        }
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(10));
    }
}
const WORKER: &str = r#"import sys,json,os
for line in sys.stdin:
 r=json.loads(line)
 if r['op']=='shutdown': break
 if r['op']=='infer':
  with open(r['directory']+'/pid','w') as f: f.write(str(os.getpid()))
 print(json.dumps({'protocol':1,'id':r['id'],'op':'ready' if r['op']=='init' else 'done','message':'canonical_pcm_range_v1'}),flush=True)
"#;
#[test]
fn resident_reuses_one_process_and_releases_it_at_job_end() {
    let (_tmp, root, snapshot) = fixture(WORKER);
    let pid;
    {
        let _scope = Scope::enter(&root, Some(&snapshot)).unwrap();
        wait(&mut start(&root, 0)).unwrap();
        wait(&mut start(&root, 1)).unwrap();
        pid = std::fs::read_to_string(root.join("unit-0/pid")).unwrap();
        assert_eq!(
            pid,
            std::fs::read_to_string(root.join("unit-1/pid")).unwrap()
        );
    }
    assert_ne!(unsafe { libc::kill(pid.parse().unwrap(), 0) }, 0);
    assert!(!enabled());
}
#[test]
fn resident_does_not_launch_for_completed_jobs_or_reuse_failed_responses() {
    let (_tmp,root,snapshot) = fixture("import sys,json\nfor line in sys.stdin:\n r=json.loads(line)\n if r['op']=='infer':\n  print(json.dumps({'protocol':1,'id':r['id']+1,'op':'done'}),flush=True)\n");
    {
        let _scope = Scope::enter(&root, Some(&snapshot)).unwrap();
        assert!(!root.join("unit-0").exists());
        let mut run = start(&root, 0);
        assert!(wait(&mut run).unwrap_err().contains("応答ID"));
        let dir = root.join("after-error");
        std::fs::create_dir(&dir).unwrap();
        assert!(Run::spawn(
            &mut OwnedCommand::new(Path::new("/unused")).unwrap(),
            "whisper",
            &dir,
            "out",
            "err"
        )
        .is_err());
    }
}
#[test]
fn resident_snapshot_is_path_free_and_worker_tampering_is_rejected() {
    let (_tmp, root, snapshot) = fixture(WORKER);
    let saved = serde_json::to_string(&snapshot).unwrap();
    assert!(!saved.contains(root.to_str().unwrap()));
    let path = snapshot.resolve(&root).unwrap().path;
    std::fs::write(&path, b"tampered").unwrap();
    assert!(snapshot.resolve(&root).is_err());
    let mut wrong = snapshot.clone();
    wrong.name = "../escape".into();
    assert!(wrong.validate().is_err());
}
#[test]
fn resident_pause_terminates_an_inflight_request_within_the_stop_budget() {
    let (_tmp,root,snapshot) = fixture("import sys,json,os,time\nfor line in sys.stdin:\n r=json.loads(line)\n if r['op']=='infer':\n  open(r['directory']+'/pid','w').write(str(os.getpid()))\n  time.sleep(120)\n");
    let start_time;
    let pid;
    {
        let _scope = Scope::enter(&root, Some(&snapshot)).unwrap();
        let mut run = start(&root, 0);
        let ready = root.join("unit-0/pid");
        let deadline = Instant::now();
        while !ready.is_file() {
            run.try_wait().unwrap();
            assert!(deadline.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(10));
        }
        pid = std::fs::read_to_string(ready).unwrap();
        start_time = Instant::now();
        // A scheduler cancellation returns without advancing the checkpoint.
    }
    assert!(start_time.elapsed() < Duration::from_secs(15));
    assert_ne!(unsafe { libc::kill(pid.parse().unwrap(), 0) }, 0);
}

pub(crate) struct Admission;
impl Admission {
    pub(crate) fn enter(snapshot: Snapshot) -> Self {
        ADMISSION.with(|v| {
            assert!(v.borrow().is_none());
            v.replace(Some(snapshot));
        });
        Self
    }
}
impl Drop for Admission {
    fn drop(&mut self) {
        ADMISSION.with(|v| {
            v.borrow_mut().take();
        });
    }
}
