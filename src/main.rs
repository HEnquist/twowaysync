extern crate notify;

use notify::{RecommendedWatcher, Watcher, RecursiveMode, PollWatcher};
use std::sync::mpsc::channel;
use std::time::Duration;
use std::env;
use std::thread::sleep;
use std::time;
use std::path::{Path, PathBuf};

#[derive(Debug)]
enum SyncAction {
    CopyFile(PathBuf, PathBuf),
    CopyDir(PathBuf, PathBuf),
    CopyMeta(PathBuf, PathBuf),
    Rename(PathBuf, PathBuf),
    DeleteFile(PathBuf),
    DeleteDir(PathBuf),
}

impl PartialEq for SyncAction {
    fn eq(&self, other: &SyncAction) -> bool {
        match (self, other) {
            (&SyncAction::CopyFile(ref a1, ref a2), &SyncAction::CopyFile(ref b1, ref b2))
            | (&SyncAction::CopyDir(ref a1, ref a2), &SyncAction::CopyDir(ref b1, ref b2))
            | (&SyncAction::CopyMeta(ref a1, ref a2), &SyncAction::CopyMeta(ref b1, ref b2))
            | (&SyncAction::Rename(ref a1, ref a2), &SyncAction::Rename(ref b1, ref b2)) => {(a1 == b1 && a2 == b2)},
            (&SyncAction::DeleteFile(ref a), &SyncAction::DeleteFile(ref b))
            | (&SyncAction::DeleteDir(ref a), &SyncAction::DeleteDir(ref b)) => (a == b),
            _ => false,
        }
    }
}

fn watch(path_a: &String, path_b: &String, interval: u64) -> notify::Result<()> {
    // Create a channel to receive the events.
    let (tx, rx) = channel();
    //let (tx_b, rx_b) = channel();

    // Automatically select the best implementation for your platform.
    // You can also access each implementation directly e.g. INotifyWatcher.
    // let mut watcher: RecommendedWatcher = (Watcher::new(tx, Duration::from_secs(2)))?;
    let mut watcher_a: PollWatcher = (Watcher::new(tx.clone(), Duration::from_secs(interval)))?;
    //let mut watcher_b: PollWatcher = (Watcher::new(tx, Duration::from_secs(interval)))?;

    // Add a path to be watched. All files and directories at that path and
    // below will be monitored for changes.
    (watcher_a.watch(path_a, RecursiveMode::Recursive))?;
    //(watcher_b.watch(path_b, RecursiveMode::Recursive))?;

    // This is a simple loop, but you may want to use more complex logic here,
    // for example to handle I/O.
    let delay = time::Duration::from_millis(1000);
    let mut events = 0;
    loop {
        match rx.try_recv() {
            Ok(event) => {
                events += 1;
                handle_event(path_a, path_b, event);
            },
            Err(e) => {
                if events>0 {
                    println!("Received {} events",events);
                    events = 0; 
                }
                //println!("watch error: {:?}", e);
                sleep(delay);
            },
        }
        //sleep(delay);
    }
}

fn handle_event(path_a: &String, path_b: &String, event: notify::DebouncedEvent) {
    println!("{:?}", event);
    match event {
        notify::DebouncedEvent::Create(e) => println!("create {:?}",SyncAction::CopyFile(PathBuf::from("/tmp"), PathBuf::from("/tmp"))),
        notify::DebouncedEvent::Write(e) => println!("write {:?}",e),
        notify::DebouncedEvent::NoticeWrite(e) => println!("notice write something"),
        notify::DebouncedEvent::NoticeRemove(e) => println!("notice write something"),
        notify::DebouncedEvent::Chmod(e) => println!("chmod something"),
        notify::DebouncedEvent::Remove(e) => println!("remove something"),
        notify::DebouncedEvent::Rename(a,b) => println!("rename something"),
        notify::DebouncedEvent::Rescan => println!("rescan"),
        notify::DebouncedEvent::Error(a,b) => println!("error"),
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let path_a = &args[1];
    let path_b = &args[2];
    let interval: u64 = args[3].parse().unwrap();
    if let Err(e) = watch(path_a, path_b, interval) {
        println!("error: {:?}", e)
    }
}



