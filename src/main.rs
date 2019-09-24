extern crate notify;

use notify::{RecommendedWatcher, Watcher, RecursiveMode, PollWatcher};
use std::sync::mpsc::channel;
use std::time::Duration;
use std::env;

fn watch(path_a: &String, path_b: &String, interval: u64) -> notify::Result<()> {
    // Create a channel to receive the events.
    let (tx, rx) = channel();
    //let (tx_b, rx_b) = channel();

    // Automatically select the best implementation for your platform.
    // You can also access each implementation directly e.g. INotifyWatcher.
    // let mut watcher: RecommendedWatcher = (Watcher::new(tx, Duration::from_secs(2)))?;
    let mut watcher_a: PollWatcher = (Watcher::new(tx.clone(), Duration::from_secs(interval)))?;
    let mut watcher_b: PollWatcher = (Watcher::new(tx, Duration::from_secs(interval)))?;

    // Add a path to be watched. All files and directories at that path and
    // below will be monitored for changes.
    (watcher_a.watch(path_a, RecursiveMode::Recursive))?;
    (watcher_b.watch(path_b, RecursiveMode::Recursive))?;

    // This is a simple loop, but you may want to use more complex logic here,
    // for example to handle I/O.
    loop {
        match rx.recv() {
            Ok(event) => handle_event(path_a, path_b, event),
            Err(e) => println!("watch error: {:?}", e),
        }

    }
}

fn handle_event(path_a: &String, path_b: &String, event: notify::DebouncedEvent) {
    println!("{:?}", event);
    match event {
        notify::DebouncedEvent::Create(e) => println!("create {:?}",e),
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



