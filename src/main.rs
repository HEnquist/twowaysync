mod datatypes;

use std::time::{Duration, SystemTime};
use std::thread::sleep;
use std::path::PathBuf;
use std::fs;
use std::fs::File;
use std::io::{Write, Read};
use filetime::FileTime;
use std::error::Error;
use std::os::unix::fs::PermissionsExt;
use std::collections::HashMap;
use walkdir::WalkDir;
use chrono::{Local, DateTime, TimeZone};
use clap::{App, Arg, ArgGroup};
use datatypes::{ChangeType, DiffItem, FileType, PathData, DirIndex, SyncAction, RunAction};

const INDEXFILENAME: &str = ".twoway.json";

fn map_dir(basepath: &PathBuf) -> Result<DirIndex,  Box<dyn Error>> {
    let current_time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_secs();
    let mut paths = HashMap::new();
    let depth = usize::max_value();
    for entry in WalkDir::new(basepath.clone())
            .follow_links(false)
            .max_depth(depth)
            .into_iter()
            .skip(1)
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            match entry.metadata() {
                Err(_) => {}
                Ok(m) => {
                    let mtime = FileTime::from_last_modification_time(&m).seconds();
                    let relpath = path.strip_prefix(basepath.to_str().unwrap_or(""))?.to_path_buf();
                    let ftype = if m.file_type().is_dir() {
                        FileType::Dir
                    }
                    else if m.file_type().is_symlink() {
                        FileType::Link
                    }
                    else {
                        FileType::File
                    };

                    //println!("insert {}",relpath.to_path_buf().display());
                    paths.insert(
                        relpath,
                        PathData {
                            mtime: mtime,
                            perms: m.permissions().mode(),
                            size: m.len(),
                            ftype: ftype,
                        },
                    );
                }
            }
        }
    let jsonpath = PathBuf::from(INDEXFILENAME);
    if paths.contains_key(&jsonpath) {
        paths.remove(&jsonpath).unwrap();
    }
    Ok(DirIndex {
        scantime: current_time,
        root: basepath.to_path_buf(),
        contents: paths,
    })
}

fn compare_dirs(dir_new: &DirIndex, dir_ref: &DirIndex) -> Result<HashMap<PathBuf, DiffItem>, Box<dyn Error>> {
    let mut diffs = HashMap::new();

    let mut dir_ref_copy = dir_ref.clone();
    for (path, pathdata_new) in dir_new.contents.iter() {
        match dir_ref.contents.get(path) {
            Some(pathdata_ref) => {
                if pathdata_new == pathdata_ref {
                    //println!("{} found, identical", path.display());
                }
                else if pathdata_new.mtime > pathdata_ref.mtime {
                    //println!("{} found, N is newer", path.display());
                    diffs.insert(
                        path.to_path_buf(),
                        DiffItem {
                            diff: ChangeType::Newer,
                            ftype: pathdata_new.ftype,
                            mtime: pathdata_new.mtime,
                        },
                    );
                }
                else if pathdata_new.mtime < pathdata_ref.mtime {
                    //println!("{} found, R is newer", path.display());
                    diffs.insert(
                        path.to_path_buf(),
                        DiffItem {
                            diff: ChangeType::Older,
                            ftype: pathdata_new.ftype,
                            mtime: pathdata_new.mtime,
                        },
                    );
                }
                else {
                    //println!("{} found, different", path.display());
                    // mode (or size, unlikely) changed
                    diffs.insert(
                        path.to_path_buf(),
                        DiffItem {
                            diff: ChangeType::Modified,
                            ftype: pathdata_new.ftype,
                            mtime: pathdata_new.mtime,
                        },
                    );
                }
                dir_ref_copy.contents.remove(path);
            }
            None => {
                //println!("{} is missing from R.", path.display());
                diffs.insert(
                        path.to_path_buf(),
                        DiffItem {
                            diff: ChangeType::NewOnly,
                            ftype: pathdata_new.ftype,
                            mtime: pathdata_new.mtime,
                        },
                );
            }
        }
    }
    for (path, pathdata_ref) in dir_ref_copy.contents.iter() {
        match dir_new.contents.get(path) {
            Some(_pathdata_new) => {
                println!("{} found in both, strange..", path.display());
            },
            None => {
                //println!("{} is missing from N.", path.display());
                diffs.insert(
                    path.to_path_buf(),
                    DiffItem {
                        diff: ChangeType::RefOnly,
                        ftype: pathdata_ref.ftype,
                        mtime: pathdata_ref.mtime,
                    },
                );
            }
        }
    }
    Ok(diffs)
}

fn solve_conflicts(diff_master: &mut HashMap<PathBuf, DiffItem>, diff_copy: &mut HashMap<PathBuf, DiffItem>) -> Result<(), Box<dyn Error>> {
    for (path, diffitem_master) in diff_master.clone().iter() {
        match diff_copy.get(path) {
            Some(diffitem_copy) => {
                match (&diffitem_master.diff, &diffitem_copy.diff) {
                    (ChangeType::Newer, ChangeType::Newer)
                    | (ChangeType::Newer, ChangeType::Older)
                    | (ChangeType::Older, ChangeType::Newer)
                    | (ChangeType::Older, ChangeType::Older)
                    | (ChangeType::NewOnly, ChangeType::NewOnly) => {
                        //check which is newer, remove oldest
                        if diffitem_master.mtime >= diffitem_copy.mtime {
                            diff_copy.remove(path);
                        }
                        else {
                            diff_master.remove(path);
                        }
                    },
                    (ChangeType::RefOnly, ChangeType::RefOnly) => {
                        //fine, remove both
                        diff_copy.remove(path);
                        diff_master.remove(path);
                    },
                    (ChangeType::Modified, ChangeType::Modified)
                    | (ChangeType::Newer, _) => {
                        //keep master 
                        diff_copy.remove(path);
                    }
                    (_, ChangeType::Newer) => {
                        //keep copy
                        diff_master.remove(path);
                    },
                    _ => {},
                }
            }
            None => {}
        }
    }
    Ok(())
}

fn translate_path(path: &PathBuf, root: &PathBuf) -> PathBuf {
    [root, path].iter().collect::<PathBuf>()
}

fn save_index(idx: &DirIndex, path: &PathBuf) -> Result<(), Box<dyn Error>> {
    let serialized = serde_json::to_string(&idx)?;
    let mut jsonpath = PathBuf::from(path);
    jsonpath.push(INDEXFILENAME);
    let mut jsonfile = File::create(jsonpath)?;
    jsonfile.write_all(serialized.as_bytes())?;
    Ok(())
}

fn load_index(path: &PathBuf) -> Result<(DirIndex), Box<dyn Error>> {
    let mut jsonpath = PathBuf::from(path);
    jsonpath.push(INDEXFILENAME);
    let mut jsonfile = File::open(jsonpath)?;
    let mut contents = String::new();
    jsonfile.read_to_string(&mut contents)?;
    let idx: DirIndex = serde_json::from_str(&contents)?;
    Ok(idx)
}

fn process_queue(mut action_queue: Vec<SyncAction>) -> Result<(), Box<dyn Error>> {
    action_queue.sort();
    for action in action_queue.drain(..) {
        println!("{}", action);
        match action.run() {
            Ok(_) => {},
            Err(e) => {
                println!("Run error {}, {:?}", e, action);
            }
        }
    }
    Ok(())
}

fn sync_diffs(diff: &HashMap<PathBuf, DiffItem>, path_src: &PathBuf, path_dest: &PathBuf, keep_all: bool) -> Result<(), Box<dyn Error>> {
    let mut actions = Vec::<SyncAction>::new();
    for (path, diffitem) in diff.iter() {
        match diffitem.diff {
            ChangeType::Newer 
            | ChangeType::NewOnly
            | ChangeType::Modified => {
                match diffitem.ftype {
                    FileType::Link => {
                        actions.push(SyncAction::CopyLink {src: translate_path(path, path_src), dest: translate_path(path, path_dest)});
                    },
                    FileType::Dir => {
                        actions.push(SyncAction::CopyDir {src: translate_path(path, path_src), dest: translate_path(path, path_dest)});
                    },
                    FileType::File => {
                        actions.push(SyncAction::CopyFile {src: translate_path(path, path_src), dest: translate_path(path, path_dest)});
                    },
                }
                actions.push(SyncAction::CopyMeta {src: translate_path(path, path_src), dest: translate_path(path, path_dest)});
            },
            ChangeType::RefOnly => {
                if keep_all {
                    match diffitem.ftype {
                        FileType::Link => {
                            actions.push(SyncAction::CopyLink {src: translate_path(path, path_dest), dest: translate_path(path, path_src)});
                        },
                        FileType::Dir => {
                            actions.push(SyncAction::CopyDir {src: translate_path(path, path_dest), dest: translate_path(path, path_src)});
                        },
                        FileType::File => {
                            actions.push(SyncAction::CopyFile {src: translate_path(path, path_dest), dest: translate_path(path, path_src)});
                        },
                    }
                    actions.push(SyncAction::CopyMeta {src: translate_path(path, path_dest), dest: translate_path(path, path_src)});
                }
                else {
                    match diffitem.ftype {
                        FileType::Dir => {
                            actions.push(SyncAction::DeleteDir {dest: translate_path(path, path_dest)});
                        },
                        _ => {
                            actions.push(SyncAction::DeleteFile {dest: translate_path(path, path_dest)});
                        },
                    }
                }
            },
            ChangeType::Older => {
                match diffitem.ftype {
                    FileType::Link => {
                        actions.push(SyncAction::CopyLink {src: translate_path(path, path_dest), dest: translate_path(path, path_src)});
                    },
                    FileType::Dir => {
                        actions.push(SyncAction::CopyDir {src: translate_path(path, path_dest), dest: translate_path(path, path_src)});
                    },
                    FileType::File => {
                        actions.push(SyncAction::CopyFile {src: translate_path(path, path_dest), dest: translate_path(path, path_src)});
                    },
                }
                actions.push(SyncAction::CopyMeta {src: translate_path(path, path_dest), dest: translate_path(path, path_src)});

            },
        }
    }
    process_queue(actions)?;
    Ok(())
}

// Main loop
fn watch(path_a: &PathBuf, path_b: &PathBuf, interval: Option<u64>, check_only: bool) -> Result<(), Box<dyn Error>> {

    let delay = match (interval, check_only) {
        (Some(ival), false) => Some(Duration::from_millis(1000*ival)),
        _ => None,
    };
    
    let mut index_a: DirIndex;
    let mut index_b: DirIndex;
    let mut index_a_new: DirIndex;
    let mut index_b_new: DirIndex;

    let mut diffs_a: HashMap<PathBuf, DiffItem>;
    let mut diffs_b: HashMap<PathBuf, DiffItem>;

    match (load_index(path_a), load_index(path_b)) {
        (Ok(idx_a), Ok(idx_b)) => {
            index_a = idx_a;
            index_b = idx_b;
            let idx_time_a: DateTime<Local> = Local.timestamp(index_a.scantime as i64, 0);
            let idx_time_b: DateTime<Local> = Local.timestamp(index_b.scantime as i64, 0);
            println!("Using indexes from {} and {}", idx_time_a, idx_time_b);
        }
        _ => {
            index_a = map_dir(path_a)?;
            index_b = map_dir(path_b)?;
            let diffs = compare_dirs(&index_a, &index_b)?;
            if check_only {
                print_diffs(&diffs);
            }
            else {
                println!("No index found, merging the contents of A and B");
                println!("This will sync all content of \n{}\nwith\n{}", path_a.display(), path_b.display());
                let reply = rprompt::prompt_reply_stdout("Proceed? y or n: ").unwrap();
                match reply.as_str() {
                    "y" => println!("Syncing..."),
                    _ => {
                        println!("Exiting");
                        return Ok(())
                    }
                }
                sync_diffs(&diffs, path_a, path_b, true)?;
                index_a = map_dir(path_a)?;
                index_b = map_dir(path_b)?;
                save_index(&index_a, &path_a)?;
                save_index(&index_b, &path_b)?;
                println!("Done");
            }
        }
    }
    match delay {
        None => return Ok(()),
        Some(delayval) => {
            let index_a_file: PathBuf = [&path_a, &PathBuf::from(INDEXFILENAME)].iter().collect();
            let index_b_file: PathBuf = [&path_b, &PathBuf::from(INDEXFILENAME)].iter().collect();
            loop {
                if fs::metadata(&index_a_file).is_ok() && fs::metadata(&index_b_file).is_ok() {
                    index_a_new = map_dir(path_a)?;
                    diffs_a = compare_dirs(&index_a_new, &index_a)?;
                    index_b_new = map_dir(path_b)?;
                    diffs_b = compare_dirs(&index_b_new, &index_b)?;
                    if fs::metadata(&index_a_file).is_ok() && fs::metadata(&index_b_file).is_ok() {
                        if !diffs_a.is_empty() || !diffs_b.is_empty() {
                            solve_conflicts(&mut diffs_a, &mut diffs_b)?;
                            sync_diffs(&diffs_a, path_a, path_b, false)?;
                            sync_diffs(&diffs_b, path_b, path_a, false)?;
                            index_a = map_dir(path_a)?;
                            index_b = map_dir(path_b)?;
                            save_index(&index_a, &path_a)?;
                            save_index(&index_b, &path_b)?;
                        }
                    }
                    else {
                        println!("One directory became unavailable while scanning!");
                    }
                }
                else {
                    println!("One directory is unavailable!");
                }
                sleep(delayval);
            }
        },
    };
}

fn print_diffs(diff: &HashMap<PathBuf, DiffItem>) {
    println!("Diffs");
    for (path, diffitem) in diff.iter() {
        println!("{}: {}", diffitem, path.display());
    }
}

fn is_valid_path(dir: String) -> Result<(), String> {
    match PathBuf::from(&dir).canonicalize() {
        Ok(_) =>  Ok(()),
        Err(_) => Err(String::from("Invalid path")),
    }
}

fn is_valid_uint(val: String) -> Result<(), String> {
    match val.parse::<usize>() {
        Ok(intval) => {
            match intval>0 {
                true => Ok(()),
                false => Err(String::from("Not a positive integer")),
            }
        }
        Err(_) => Err(String::from("Not a number")),
    }
}

fn main() {
    let matches = App::new("TwoWaySync")
                    .version("0.1.1")
                    .author("Henrik Enquist <henrik.enquist@gmail.com>")
                    .about("Sync two directories")
                    .arg(Arg::with_name("interval")
                                .short("w")
                                .long("watch")
                                .help("Interval in seconds to watch for changes")
                                .validator(is_valid_uint)
                                .takes_value(true))
                    .arg(Arg::with_name("single")
                                .short("s")
                                .help("Do a single sync only"))   
                    .arg(Arg::with_name("check")
                                .short("c")
                                .help("Compare and show diff (default)"))   
                    .group(ArgGroup::with_name("sync")
                                .args(&["check", "single", "interval"]))      
                    .arg(Arg::with_name("dir_a")
                                    .help("First directory")
                                    .required(true)
                                    .validator(is_valid_path)
                                    .index(1))
                    .arg(Arg::with_name("dir_b")
                                    .help("Second directory")
                                    .required(true)
                                    .validator(is_valid_path)
                                    .index(2))
                    .get_matches();
    
    let check_only = matches.is_present("check");

    let path_a = match matches.value_of("dir_a") {
        Some(path) => PathBuf::from(&path).canonicalize().unwrap(),
        _ => PathBuf::new(),
    };

    let path_b = match matches.value_of("dir_b") {
        Some(path) => PathBuf::from(&path).canonicalize().unwrap(),
        _ => PathBuf::new(),
    };

    let interval = match matches.value_of("interval") {
        Some(i) => Some(i.parse::<u64>().unwrap()),
        _ => None,
    };


    match watch(&path_a, &path_b, interval, check_only) {
        Ok(_) => {},
        Err(e) => {
            println!("Run error {}", e);
        }
    }
}



