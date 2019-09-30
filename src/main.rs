extern crate notify;

use notify::{RecommendedWatcher, Watcher, RecursiveMode, PollWatcher};
use std::sync::mpsc::channel;
use std::time::Duration;
use std::env;
use std::thread::sleep;
use std::time;
use std::path::{Path, PathBuf};
use std::cmp::Ordering;

#[derive(Debug)]
enum SyncAction {
    CopyFile {src: PathBuf, dest: PathBuf},
    CopyDir {src: PathBuf, dest: PathBuf},
    CopyMeta {src: PathBuf, dest: PathBuf},
    Rename {src: PathBuf, dest: PathBuf},
    DeleteFile {src: PathBuf},
    DeleteDir {src: PathBuf},
}

trait Prio {
    fn prio(&self) -> usize;
}

impl Prio for SyncAction {
    fn prio(&self) -> usize {
        match self {
            &SyncAction::CopyFile {src: _, dest: _} => 2,
            &SyncAction::CopyDir {src: _, dest: _} => 1,
            &SyncAction::CopyMeta {src: _, dest: _} => 6,
            &SyncAction::Rename {src: _, dest: _} => 3,
            &SyncAction::DeleteFile {src: _} => 4,
            &SyncAction::DeleteDir {src: _} => 5,
        }
    }
}

impl PartialEq for SyncAction {
    fn eq(&self, other: &SyncAction) -> bool {
        match (self, other) {
            (&SyncAction::CopyFile {src: ref src_a, dest: ref dest_a}, &SyncAction::CopyFile {src: ref src_b, dest: ref dest_b})
            | (&SyncAction::CopyDir {src: ref src_a, dest: ref dest_a}, &SyncAction::CopyDir {src: ref src_b, dest: ref dest_b})
            | (&SyncAction::CopyMeta {src: ref src_a, dest: ref dest_a}, &SyncAction::CopyMeta {src: ref src_b, dest: ref dest_b})
            | (&SyncAction::Rename {src: ref src_a, dest: ref dest_a}, &SyncAction::Rename {src: ref src_b, dest: ref dest_b}) => {(src_a == src_b && dest_a == dest_b)},
            (&SyncAction::DeleteFile {src: ref src_a}, &SyncAction::DeleteFile {src: ref src_b})
            | (&SyncAction::DeleteDir {src: ref src_a}, &SyncAction::DeleteDir {src: ref src_b}) => (src_a == src_b),
            _ => false,
        }
    }
}

impl PartialOrd for SyncAction {
    fn partial_cmp(&self, other: &SyncAction) -> Option<Ordering> {
        match (self, other) {
            (&SyncAction::CopyFile {src: ref src_a, dest: ref dest_a}, &SyncAction::CopyFile {src: ref src_b, dest: ref dest_b})
            | (&SyncAction::CopyDir {src: ref src_a, dest: ref dest_a}, &SyncAction::CopyDir {src: ref src_b, dest: ref dest_b}) => src_a.iter().count().partial_cmp(&src_b.iter().count()),
            | (&SyncAction::CopyMeta {src: ref src_a, dest: ref dest_a}, &SyncAction::CopyMeta {src: ref src_b, dest: ref dest_b})
            | (&SyncAction::Rename {src: ref src_a, dest: ref dest_a}, &SyncAction::Rename {src: ref src_b, dest: ref dest_b}) => src_b.iter().count().partial_cmp(&src_a.iter().count()),
            (&SyncAction::DeleteFile {src: ref src_a}, &SyncAction::DeleteFile {src: ref src_b})
            | (&SyncAction::DeleteDir {src: ref src_a}, &SyncAction::DeleteDir {src: ref src_b}) => src_b.iter().count().partial_cmp(&src_a.iter().count()),
            _ => self.prio().partial_cmp(&other.prio()),
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
        notify::DebouncedEvent::Create(e) => println!("create {:?}",SyncAction::CopyFile {src: PathBuf::from("/tmp"), dest: PathBuf::from("/tmp")}),
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

    let mut actions = Vec::new();
    actions.push(SyncAction::CopyFile {src: PathBuf::from("/tmp/dir"), dest: PathBuf::from("/tmp/dir")});
    actions.push(SyncAction::CopyFile {src: PathBuf::from("/tmp"), dest: PathBuf::from("/tmp")});
    actions.push(SyncAction::CopyDir {src: PathBuf::from("/tmp"), dest: PathBuf::from("/tmp")});
    actions.push(SyncAction::CopyMeta {src: PathBuf::from("/tmp"), dest: PathBuf::from("/tmp")});
    actions.push(SyncAction::DeleteFile {src: PathBuf::from("/tmp")});

    actions.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!("{:?}", actions);

    if let Err(e) = watch(path_a, path_b, interval) {
        println!("error: {:?}", e)
    }
}



