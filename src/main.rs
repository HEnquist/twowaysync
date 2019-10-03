extern crate notify;

use notify::{Watcher, RecursiveMode, PollWatcher};
use std::sync::mpsc::channel;
use std::time::Duration;
use std::env;
use std::thread::sleep;
use std::time;
use std::path::PathBuf;
use std::cmp::Ordering;
use std::fs;
use filetime::FileTime;
use std::error::Error;
use std::fmt;

#[derive(Debug)]
struct SyncError {
    details: String
}

impl SyncError {
    fn new(msg: &str) -> SyncError {
        SyncError{details: msg.to_string()}
    }
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f,"{}",self.details)
    }
}

impl Error for SyncError {
    fn description(&self) -> &str {
        &self.details
    }
}

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

impl Ord for SyncAction {
    fn cmp(&self, other: &SyncAction) -> Ordering {
        match (self, other) {
            (&SyncAction::CopyFile {src: ref src_a, dest: _}, &SyncAction::CopyFile {src: ref src_b, dest: _})
            | (&SyncAction::CopyDir {src: ref src_a, dest: _}, &SyncAction::CopyDir {src: ref src_b, dest: _}) => src_a.iter().count().cmp(&src_b.iter().count()),
            | (&SyncAction::CopyMeta {src: ref src_a, dest: _}, &SyncAction::CopyMeta {src: ref src_b, dest: _})
            | (&SyncAction::Rename {src: ref src_a, dest: _}, &SyncAction::Rename {src: ref src_b, dest: _}) => src_b.iter().count().cmp(&src_a.iter().count()),
            (&SyncAction::DeleteFile {src: ref src_a}, &SyncAction::DeleteFile {src: ref src_b})
            | (&SyncAction::DeleteDir {src: ref src_a}, &SyncAction::DeleteDir {src: ref src_b}) => src_b.iter().count().cmp(&src_a.iter().count()),
            _ => self.prio().cmp(&other.prio()),
        }
    }
}

impl Eq for SyncAction {}

impl PartialOrd for SyncAction {
    fn partial_cmp(&self, other: &SyncAction) -> Option<Ordering> {
        Some(self.cmp(&other))
    }
}

trait RunAction {
    fn run(&self) -> Result<(), Box<dyn Error>>;
}

impl RunAction for SyncAction {
    fn run(&self) -> Result<(), Box<dyn Error>> {
        match self {
            SyncAction::CopyFile {src, dest} => {
                let _bytescopied = fs::copy(&src, &dest)?;
                Ok(())
            },
            SyncAction::CopyDir {src: _, dest} => {
                fs::create_dir(&dest)?;
                Ok(())
            },
            SyncAction::CopyMeta {src, dest} => {
                let perms = fs::metadata(&src).unwrap().permissions();
                fs::set_permissions(&dest, perms)?;
                let attr = fs::metadata(&src).unwrap();
                let mtime = FileTime::from_last_modification_time(&attr);
                let atime = FileTime::from_last_access_time(&attr);
                let _res = filetime::set_file_times(&dest, atime, mtime);
                Ok(())
            },
            SyncAction::Rename {src, dest} => {
                fs::rename(&src, &dest)?;
                Ok(())
            },
            SyncAction::DeleteFile {src} => {
                fs::remove_file(&src)?;
                Ok(())
            },
            SyncAction::DeleteDir {src} => {
                fs::remove_dir(&src)?;
                Ok(())
            }
        }
    }
}



fn translate_path(src_path: &PathBuf, src_base: &PathBuf, dest_base: &PathBuf) -> Result<(PathBuf), SyncError> {
    let mut dest_path = dest_base.clone();
    if src_path.starts_with(&src_base) {
        for part in src_path.iter().skip(src_base.iter().count()) {
            dest_path.push(part);
        }
        Ok(dest_path)
    }
    else {
        Err(SyncError::new("bad path"))
    }
}




fn watch(path_a: &PathBuf, path_b: &PathBuf, interval: u64) -> notify::Result<()> {
    // Create a channel to receive the events.
    let (tx_a, rx_a) = channel();
    let (tx_b, rx_b) = channel();
    let (tx_a_parent, rx_a_parent) = channel();
    let (tx_b_parent, rx_b_parent) = channel();

    let mut watcher_a: PollWatcher = (Watcher::new(tx_a, Duration::from_secs(interval)))?;
    let mut watcher_b: PollWatcher = (Watcher::new(tx_b, Duration::from_secs(interval)))?;
    let mut watcher_a_parent: PollWatcher = (Watcher::new(tx_a_parent, Duration::from_secs(interval/2)))?;
    let mut watcher_b_parent: PollWatcher = (Watcher::new(tx_b_parent, Duration::from_secs(interval/2)))?;

    (watcher_a.watch(path_a, RecursiveMode::Recursive))?;
    (watcher_b.watch(path_b, RecursiveMode::Recursive))?;
    (watcher_a_parent.watch(path_a, RecursiveMode::NonRecursive))?;
    (watcher_b_parent.watch(path_b, RecursiveMode::NonRecursive))?;

    let delay = time::Duration::from_millis(1000);
    //let mut events_a = 0;
    //let mut events_b = 0;
    let mut action_queue_a = Vec::new();
    let mut action_queue_b = Vec::new();
    let mut path_a_ok = true;
    let mut path_b_ok = true;


    loop {
        while let Ok(event) = rx_a_parent.try_recv() {
            println!("DirA {:?}", event);
            match event {
                notify::DebouncedEvent::Error(_a,_b) => {
                    if path_a_ok {
                        println!("stop watching A due to error");
                        let _res = watcher_a.unwatch(path_a);
                        path_a_ok = false;
                    }
                },
                notify::DebouncedEvent::Create(path) => {
                    if &path == path_a {
                        if !path_a_ok {
                            println!("restart watching A");
                            //clear event queues
                            while let Ok(_) = rx_a.try_recv() {}
                            action_queue_a.clear();
                            watcher_a.watch(path_a, RecursiveMode::Recursive)?;
                            path_a_ok = true;
                        }
                    } 
                },
                _ => {}
            }
        }
        while let Ok(event) = rx_b_parent.try_recv() {
            println!("DirB {:?}", event);
            match event {
                notify::DebouncedEvent::Error(_a,_b) => {
                    if path_b_ok {
                        println!("stop watching B due to error");
                        let _res = watcher_b.unwatch(path_b);
                        path_b_ok = false;
                    }
                },
                notify::DebouncedEvent::Create(path) => {
                    if &path == path_b {
                        if !path_b_ok {
                            println!("restart watching B");
                            //clear event queues
                            while let Ok(_) = rx_b.try_recv() {}
                            action_queue_b.clear();
                            watcher_b.watch(path_b, RecursiveMode::Recursive)?;
                            path_b_ok = true;
                        }
                    } 
                },
                _ => {}
            }
        }
        if path_a_ok {
            while let Ok(event) = rx_a.try_recv() {
                match queue_actions(&mut action_queue_a, path_a, path_b, event) {
                    Ok(_) => {},
                    Err(e) => {
                        println!("Error adding to queue A {}", e);
                    }
                }
            }
            if action_queue_a.len()>0 && path_b_ok {
                let _res = process_queue(&mut action_queue_a, path_b, &mut watcher_b);
            }
        }
        if path_b_ok {
            while let Ok(event) = rx_b.try_recv() {
                match queue_actions(&mut action_queue_b, path_b, path_a, event) {
                    Ok(_) => {},
                    Err(e) => {
                        println!("Error adding to queue B {}", e);
                    }
                }
            }
            if action_queue_b.len()>0 && path_a_ok {
                let _res = process_queue(&mut action_queue_b, path_a, &mut watcher_a);
            }
        }
        //if action_queue_a.len()==0 && action_queue_b.len()==0 {
        sleep(delay);
    }
}

fn process_queue<T: Watcher>(action_queue: &mut Vec<SyncAction>, target_path: &PathBuf , target_watcher: &mut T) -> Result<(), Box<dyn Error>> {
    target_watcher.unwatch(target_path)?;
    action_queue.sort();
    for action in action_queue.drain(..) {
        println!("Running {:?}", action);
        match action.run() {
            Ok(_) => {},
            Err(e) => {
                println!("Run error {}, {:?}", e, action);
            }
        }
    }
    target_watcher.watch(target_path, RecursiveMode::Recursive)?;
    Ok(())
}

fn queue_actions(action_queue: &mut Vec<SyncAction>, path_a: &PathBuf, path_b: &PathBuf, event: notify::DebouncedEvent) -> Result<(), Box<dyn Error>> {
    println!("{:?}", event);
    match event {
        notify::DebouncedEvent::Create(path) => {
            if path.is_dir() {
                println!("create dir");
                action_queue.push(SyncAction::CopyDir {src: path.clone(), dest: translate_path(&path, &path_a, &path_b)?});
            }
            else {
                println!("create file");
                action_queue.push(SyncAction::CopyFile {src: path.clone(), dest: translate_path(&path, &path_a, &path_b)?});
            }
            action_queue.push(SyncAction::CopyMeta {src: path.clone(), dest: translate_path(&path, &path_a, &path_b)?});
            Ok(())
        },
        notify::DebouncedEvent::Write(path) => {
            if path.is_dir() {
                println!("write dir");
                //action_queue.push(SyncAction::CopyDir {src: path.clone(), dest: translate_path(&path, &path_a, &path_b)?});
            }
            else {
                println!("write file");
                action_queue.push(SyncAction::CopyFile {src: path.clone(), dest: translate_path(&path, &path_a, &path_b)?});
            }
            action_queue.push(SyncAction::CopyMeta {src: path.clone(), dest: translate_path(&path, &path_a, &path_b)?});
            Ok(())
        },
        notify::DebouncedEvent::NoticeWrite(_path) => {
            println!("notice write something");
            Ok(())
        },
        notify::DebouncedEvent::NoticeRemove(_path) => {
            println!("notice write something");
            Ok(())
        },
        notify::DebouncedEvent::Chmod(path) => {
            println!("chmod something");
            action_queue.push(SyncAction::CopyMeta {src: path.clone(), dest: translate_path(&path, &path_a, &path_b)?});
            Ok(())
        },
        notify::DebouncedEvent::Remove(path) => {
            if translate_path(&path, &path_a, &path_b)?.is_dir() {
                println!("delete dir");
                action_queue.push(SyncAction::DeleteDir {src: translate_path(&path, &path_a, &path_b)?});
            }
            else {
                println!("delete file");
                action_queue.push(SyncAction::DeleteFile {src: translate_path(&path, &path_a, &path_b)?});
            }
            Ok(())
        },
        notify::DebouncedEvent::Rename(path_src, path_dest) => {
            println!("rename something");
            action_queue.push(SyncAction::Rename {src: translate_path(&path_src, &path_a, &path_b)?, dest: translate_path(&path_dest, &path_a, &path_b)?});
            action_queue.push(SyncAction::CopyMeta {src: path_dest.clone(), dest: translate_path(&path_dest, &path_a, &path_b)?});
            Ok(())
        },
        notify::DebouncedEvent::Rescan => {
            println!("rescan");
            Ok(())
        },
        notify::DebouncedEvent::Error(_a,_b) => {
            println!("error");
            Ok(())
        }
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let path_a = PathBuf::from(&args[1]).canonicalize().unwrap();
    let path_b = PathBuf::from(&args[2]).canonicalize().unwrap();
    let interval: u64 = args[3].parse().unwrap();

    if let Err(e) = watch(&path_a, &path_b, interval) {
        println!("error: {:?}", e)
    }
}



