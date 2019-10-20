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
use globset::{Glob, GlobSetBuilder, GlobSet};

const INDEXFILENAME: &str = ".twoway.json";

fn map_dir(basepath: &PathBuf, exclude_globs: &GlobSet) -> Result<DirIndex,  Box<dyn Error>> {
    let basepath_str = basepath.to_str().unwrap();
    let current_time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_secs();
    let mut paths = HashMap::new();
    let depth = usize::max_value();
    for direntry in WalkDir::new(basepath.clone())
            .follow_links(false)
            .max_depth(depth)
            .into_iter()
            .filter_entry(|e| {
                !exclude_globs.is_match(e.path().strip_prefix(&basepath_str).unwrap())
            })
            .skip(1)
    {
        let entry = direntry?; 
        let path = entry.path();
        let m = entry.metadata()?;
        let mtime = FileTime::from_last_modification_time(&m).seconds();
        let relpath = path.strip_prefix(&basepath_str).unwrap().to_path_buf();
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
                        DiffItem::new(ChangeType::Newer, pathdata_new.ftype, pathdata_new.mtime),
                    );
                }
                else if pathdata_new.mtime < pathdata_ref.mtime {
                    //println!("{} found, R is newer", path.display());
                    diffs.insert(
                        path.to_path_buf(),
                        DiffItem::new(ChangeType::Older, pathdata_new.ftype, pathdata_new.mtime),
                    );
                }
                else {
                    //println!("{} found, different", path.display());
                    // mode (or size, unlikely) changed
                    diffs.insert(
                        path.to_path_buf(),
                        DiffItem::new(ChangeType::Modified, pathdata_new.ftype, pathdata_new.mtime),
                    );
                }
                dir_ref_copy.contents.remove(path);
            }
            None => {
                //println!("{} is missing from R.", path.display());
                diffs.insert(
                        path.to_path_buf(),
                        DiffItem::new(ChangeType::NewOnly, pathdata_new.ftype, pathdata_new.mtime),
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
                    DiffItem::new(ChangeType::RefOnly, pathdata_ref.ftype, pathdata_ref.mtime),
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

fn append_base_path(path: &PathBuf, root: &PathBuf) -> PathBuf {
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
                println!("Action run error {}, {}", e, action);
            }
        }
    }
    Ok(())
}

fn sync_diffs(diff: &HashMap<PathBuf, DiffItem>, path_src: &PathBuf, path_dest: &PathBuf, keep_all: bool) -> Result<(), Box<dyn Error>> {
    let mut actions = Vec::<SyncAction>::new();
    for (path, diffitem) in diff.iter() {
        match (&diffitem.diff, keep_all) {
            (&ChangeType::Newer, _) 
            | (&ChangeType::NewOnly, _)
            | (&ChangeType::Modified, _) => {
                let src = append_base_path(path, path_src);
                let dest = append_base_path(path, path_dest);
                actions.push(match diffitem.ftype {
                    FileType::Link => SyncAction::CopyLink {src: src.to_path_buf(), dest: dest.to_path_buf()},
                    FileType::Dir => SyncAction::CopyDir {src: src.to_path_buf(), dest: dest.to_path_buf()},
                    FileType::File => SyncAction::CopyFile {src: src.to_path_buf(), dest: dest.to_path_buf()},
                });
                actions.push(SyncAction::CopyMeta  {src: src.to_path_buf(), dest: dest.to_path_buf()});
            },
            (&ChangeType::RefOnly, false) => {
                let dest = append_base_path(path, path_dest);
                actions.push(match diffitem.ftype {
                    FileType::Dir => SyncAction::DeleteDir {dest: dest.to_path_buf()},
                    _ => SyncAction::DeleteFile {dest: dest.to_path_buf()},
                });
            },
            (&ChangeType::Older, _)
            | (&ChangeType::RefOnly, true) => {
                let src = append_base_path(path, path_dest);
                let dest = append_base_path(path, path_src);
                actions.push(match diffitem.ftype {
                    FileType::Link => SyncAction::CopyLink {src: src.to_path_buf(), dest: dest.to_path_buf()},
                    FileType::Dir => SyncAction::CopyDir {src: src.to_path_buf(), dest: dest.to_path_buf()},
                    FileType::File => SyncAction::CopyFile {src: src.to_path_buf(), dest: dest.to_path_buf()},
                });
                actions.push(SyncAction::CopyMeta  {src: src.to_path_buf(), dest: dest.to_path_buf()});
            },
        }
    }
    process_queue(actions)?;
    Ok(())
}

// Main loop
fn watch(path_a: &PathBuf, path_b: &PathBuf, interval: Option<u64>, check_only: bool, exclude_globs: GlobSet) -> Result<(), Box<dyn Error>> {

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
            index_a = map_dir(path_a, &exclude_globs)?;
            index_b = map_dir(path_b, &exclude_globs)?;
            let diffs = compare_dirs(&index_a, &index_b)?;
            if check_only {
                print_diffs(&diffs);
                return Ok(())
            }
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
            index_a = map_dir(path_a, &exclude_globs)?;
            index_b = map_dir(path_b, &exclude_globs)?;
            save_index(&index_a, &path_a)?;
            save_index(&index_b, &path_b)?;
            println!("Done");
        }
    }
    if let Some(delayval) = delay {
        let index_a_file: PathBuf = [&path_a, &PathBuf::from(INDEXFILENAME)].iter().collect();
        let index_b_file: PathBuf = [&path_b, &PathBuf::from(INDEXFILENAME)].iter().collect();
        loop {
            sleep(delayval);
            if fs::metadata(&index_a_file).is_ok() && fs::metadata(&index_b_file).is_ok() {
                if let (Ok(idx_a), Ok(idx_b)) = (map_dir(path_a, &exclude_globs), map_dir(path_b, &exclude_globs)) {
                    index_a_new = idx_a;
                    index_b_new = idx_b;
                }
                else {
                    println!("One scan task encountered an error!");
                    continue;
                }
                diffs_a = compare_dirs(&index_a_new, &index_a)?;
                diffs_b = compare_dirs(&index_b_new, &index_b)?;
                if !diffs_a.is_empty() || !diffs_b.is_empty() {    
                    if fs::metadata(&index_a_file).is_ok() && fs::metadata(&index_b_file).is_ok() {
                        solve_conflicts(&mut diffs_a, &mut diffs_b)?;
                        sync_diffs(&diffs_a, path_a, path_b, false)?;
                        sync_diffs(&diffs_b, path_b, path_a, false)?;
                        index_a = map_dir(path_a, &exclude_globs)?;
                        index_b = map_dir(path_b, &exclude_globs)?;
                        save_index(&index_a, &path_a)?;
                        save_index(&index_b, &path_b)?;
                    }
                    else {
                        println!("One directory became unavailable while scanning!");
                    }
                }
            }
            else {
                println!("One directory is unavailable!");
            }
        }
    }
    else {
        return Ok(());
    }
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

fn is_valid_pattern(patt: String) -> Result<(), String> {
    match Glob::new(&patt) {
        Ok(_) => Ok(()),
        Err(_) => Err(String::from("Invalid pattern")),
    }
}

fn main() {
    let matches = App::new("TwoWaySync")
                    .version("0.1.2")
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
                    .arg(Arg::with_name("exclude")
                                .short("e")
                                .long("exclude")
                                .takes_value(true)
                                .number_of_values(1)
                                .multiple(true)
                                .validator(is_valid_pattern)
                                .help("Exclude files and dirs matching pattern"))
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

    let mut builder = GlobSetBuilder::new();
    if let Some(excludes) = matches.values_of("exclude") {
        for excl in excludes {
            builder.add(Glob::new(&excl).unwrap());
        }
    }
    builder.add(Glob::new(INDEXFILENAME).unwrap());
    let exclude_globs = builder.build().unwrap();

    match watch(&path_a, &path_b, interval, check_only, exclude_globs) {
        Ok(_) => {},
        Err(e) => {
            println!("Watch loop returned an error {}", e);
        }
    }
}



