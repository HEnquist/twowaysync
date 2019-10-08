use std::time::{Duration, SystemTime};
use std::env;
use std::thread::sleep;
use std::time;
use std::path::PathBuf;
use std::cmp::Ordering;
use std::fs;
use filetime::FileTime;
use std::error::Error;
use std::fmt;
use std::os::unix::fs::PermissionsExt;
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use walkdir::WalkDir;

#[derive(Clone, Debug, PartialEq)]
enum ChangeType {
    Newer,
    Older,
    Created,
    Deleted,
}

#[derive(Clone, Debug)]
struct DiffItem {
    diff: ChangeType,
    ftype: FileType,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
enum FileType {
    File,
    Dir,
    Link,
}

#[derive(Clone, Serialize, Deserialize)]
struct PathData {
    mtime: i64,
    perms: u32,
    size: u64,
    ftype: FileType,
}

#[derive(Clone, Serialize, Deserialize)]
struct DirIndex {
    scantime: u64,
    root: PathBuf,
    contents: HashMap<PathBuf, PathData>,
}

impl PartialEq for PathData {
    fn eq(&self, other: &PathData) -> bool {
        self.mtime == other.mtime && self.perms == other.perms && self.size == other.size && self.ftype == other.ftype
    }
}

impl Eq for PathData {}


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
    CopyLink {src: PathBuf, dest: PathBuf},
    CopyMeta {src: PathBuf, dest: PathBuf},
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
            &SyncAction::CopyLink {src: _, dest: _} => 4,
            &SyncAction::CopyMeta {src: _, dest: _} => 7,
            &SyncAction::DeleteFile {src: _} => 5,
            &SyncAction::DeleteDir {src: _} => 6,
        }
    }
}

impl PartialEq for SyncAction {
    fn eq(&self, other: &SyncAction) -> bool {
        match (self, other) {
            (&SyncAction::CopyFile {src: ref src_a, dest: ref dest_a}, &SyncAction::CopyFile {src: ref src_b, dest: ref dest_b})
            | (&SyncAction::CopyDir {src: ref src_a, dest: ref dest_a}, &SyncAction::CopyDir {src: ref src_b, dest: ref dest_b})
            | (&SyncAction::CopyMeta {src: ref src_a, dest: ref dest_a}, &SyncAction::CopyMeta {src: ref src_b, dest: ref dest_b})
            | (&SyncAction::CopyLink {src: ref src_a, dest: ref dest_a}, &SyncAction::CopyLink {src: ref src_b, dest: ref dest_b}) => {(src_a == src_b && dest_a == dest_b)},
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
            | (&SyncAction::CopyLink {src: ref src_a, dest: _}, &SyncAction::CopyLink {src: ref src_b, dest: _})
            | (&SyncAction::CopyDir {src: ref src_a, dest: _}, &SyncAction::CopyDir {src: ref src_b, dest: _}) => src_a.iter().count().cmp(&src_b.iter().count()),
            (&SyncAction::CopyMeta {src: ref src_a, dest: _}, &SyncAction::CopyMeta {src: ref src_b, dest: _}) => src_b.iter().count().cmp(&src_a.iter().count()),
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

impl fmt::Display for SyncAction {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            SyncAction::CopyFile {src, dest: _} => write!(f,"CopyFile: {}",src.display()),
            SyncAction::CopyDir {src, dest: _} => write!(f,"CopyDir: {}",src.display()),
            SyncAction::CopyMeta {src, dest: _} => write!(f,"CopyMeta: {}",src.display()),
            SyncAction::CopyLink {src, dest: _} => write!(f,"CopyLink: {}",src.display()),
            SyncAction::DeleteFile {src} => write!(f,"DeleteFile: {}",src.display()),
            SyncAction::DeleteDir {src} => write!(f,"DeleteDir: {}",src.display()),
        }
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
                let perms = fs::metadata(&src)?.permissions();
                fs::set_permissions(&dest, perms)?;
                let attr = fs::metadata(&src)?;
                let mtime = FileTime::from_last_modification_time(&attr);
                let atime = FileTime::from_last_access_time(&attr);
                let _res = filetime::set_file_times(&dest, atime, mtime);
                Ok(())
            },
            SyncAction::CopyLink {src, dest} => {
                //let attr = fs::symlink_metadata(src)?;
                let target = fs::read_link(src)?;
                std::os::unix::fs::symlink(target, dest)?;
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

fn map_dir(basepath: &PathBuf) -> Result<DirIndex,  Box<Error>> {
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
                Err(e) => {}
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
    Ok(DirIndex {
        scantime: current_time,
        root: basepath.to_path_buf(),
        contents: paths,
    })
}

fn compare_dirs(dir_a: &DirIndex, dir_b: &DirIndex) -> Result<HashMap<PathBuf, DiffItem>, Box<dyn Error>> {
    let mut diffs = HashMap::new();

    let mut dir_b_copy = dir_b.clone();
    for (path, pathdata_a) in dir_a.contents.iter() {
        match dir_b.contents.get(path) {
            Some(pathdata_b) => {
                if pathdata_a == pathdata_b {
                    //println!("{} found, identical", path.display());
                }
                else if pathdata_a.mtime > pathdata_b.mtime {
                    //println!("{} found, A is newer", path.display());
                    // copy A to B
                    diffs.insert(
                        path.to_path_buf(),
                        DiffItem {
                            diff: ChangeType::Newer,
                            ftype: pathdata_a.ftype,
                        },
                    );
                }
                else if pathdata_a.mtime < pathdata_b.mtime {
                    //println!("{} found, B is newer", path.display());
                    // copy B to A
                    diffs.insert(
                        path.to_path_buf(),
                        DiffItem {
                            diff: ChangeType::Older,
                            ftype: pathdata_a.ftype,
                        },
                    );
                }
                else {
                    //println!("{} found, different", path.display());
                    // mode changed
                    diffs.insert(
                        path.to_path_buf(),
                        DiffItem {
                            diff: ChangeType::Newer,
                            ftype: pathdata_a.ftype,
                        },
                    );
                }
                dir_b_copy.contents.remove(path);
            }
            None => {
                //println!("{} is missing from B.", path.display());
                // copy A to B
                diffs.insert(
                        path.to_path_buf(),
                        DiffItem {
                            diff: ChangeType::Created,
                            ftype: pathdata_a.ftype,
                        },
                );
            }
        }
    }
    for (path, pathdata_b) in dir_b_copy.contents.iter() {
        match dir_a.contents.get(path) {
            Some(pathdata_a) => {
                println!("{} found in both, strange..", path.display());
            },
            None => {
                //println!("{} is missing from A.", path.display());
                // copy B to A
                diffs.insert(
                    path.to_path_buf(),
                    DiffItem {
                        diff: ChangeType::Deleted,
                        ftype: pathdata_b.ftype,
                    },
                );
            }
        }
    }
    Ok(diffs)
}


fn translate_path(path: &PathBuf, root: &PathBuf) -> PathBuf {
    let mut dest_path = root.clone();
    dest_path.push(path);
    dest_path
}



fn watch(path_a: &PathBuf, path_b: &PathBuf, interval: u64) {

    let delay = time::Duration::from_millis(1000*interval);
    let mut action_queue_a = Vec::<SyncAction>::new();
    let mut action_queue_b = Vec::<SyncAction>::new();
    let mut path_a_ok = true;
    let mut path_b_ok = true;

    let mut index_a = map_dir(path_a).unwrap();
    let mut index_b = map_dir(path_b).unwrap();
    let mut index_a_new: DirIndex;
    let mut index_b_new: DirIndex;
    let mut diffs_a: HashMap<PathBuf, DiffItem>;
    let mut diffs_b: HashMap<PathBuf, DiffItem>;
    let diffs = compare_dirs(&index_a, &index_b).unwrap();
    println!("diffs {:?}", diffs);


    loop {
        index_a_new = map_dir(path_a).unwrap();
        diffs_a = compare_dirs(&index_a_new, &index_a).unwrap();
        index_a = index_a_new;
        println!("A changes {:?}", diffs_a);
        index_b_new = map_dir(path_b).unwrap();
        diffs_b = compare_dirs(&index_b_new, &index_b).unwrap();
        index_b = index_b_new;
        println!("B changes {:?}", diffs_b);
        //check for changes in A and queue actions
        //check for changes in B and queue actions
        //check for conflicts
        //if changes in A:
        //    process queue A
        //    update index A 
        //if changes in B:
        //    process queue B
        //    update index B 
        sleep(delay);
    }
}

fn process_queue(action_queue: &mut Vec<SyncAction>) -> Result<(), Box<dyn Error>> {
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

//fn queue_actions(action_queue: &mut Vec<SyncAction>, path_a: &PathBuf, path_b: &PathBuf, event: notify::DebouncedEvent) -> Result<(), Box<dyn Error>> {
//    println!("Event: {:?}", event);
//    match event {
//        notify::DebouncedEvent::Create(path) => {
//            let attr = fs::symlink_metadata(&path)?;
//            if attr.file_type().is_symlink() {
//                action_queue.push(SyncAction::CopyLink {src: path.clone(), dest: translate_path(&path, &path_a, &path_b)?});
//            }
//            else if attr.is_dir() {
//                //println!("create dir");
//                action_queue.push(SyncAction::CopyDir {src: path.clone(), dest: translate_path(&path, &path_a, &path_b)?});
//            }
//            else {
//                //println!("create file");
//                action_queue.push(SyncAction::CopyFile {src: path.clone(), dest: translate_path(&path, &path_a, &path_b)?});
//            }
//            action_queue.push(SyncAction::CopyMeta {src: path.clone(), dest: translate_path(&path, &path_a, &path_b)?});
//            Ok(())
//        },
//        notify::DebouncedEvent::Write(path) => {
//            if path.is_dir() {
//                //println!("write dir");
//                //action_queue.push(SyncAction::CopyDir {src: path.clone(), dest: translate_path(&path, &path_a, &path_b)?});
//            }
//            else {
//                //println!("write file");
//                action_queue.push(SyncAction::CopyFile {src: path.clone(), dest: translate_path(&path, &path_a, &path_b)?});
//            }
//            action_queue.push(SyncAction::CopyMeta {src: path.clone(), dest: translate_path(&path, &path_a, &path_b)?});
//            Ok(())
//        },
//        notify::DebouncedEvent::NoticeWrite(_path) => {
//            //println!("notice write something");
//            Ok(())
//        },
//        notify::DebouncedEvent::NoticeRemove(_path) => {
//            //println!("notice write something");
//            Ok(())
//        },
//        notify::DebouncedEvent::Chmod(path) => {
//            //println!("chmod something");
//            action_queue.push(SyncAction::CopyMeta {src: path.clone(), dest: translate_path(&path, &path_a, &path_b)?});
//            Ok(())
//        },
//        notify::DebouncedEvent::Remove(path) => {
//            if translate_path(&path, &path_a, &path_b)?.is_dir() {
//                //println!("delete dir");
//                action_queue.push(SyncAction::DeleteDir {src: translate_path(&path, &path_a, &path_b)?});
//            }
//            else {
//                //println!("delete file");
//                action_queue.push(SyncAction::DeleteFile {src: translate_path(&path, &path_a, &path_b)?});
//            }
//            Ok(())
//        },
//        notify::DebouncedEvent::Rename(path_src, path_dest) => {
//            //println!("rename something");
//            action_queue.push(SyncAction::Rename {src: translate_path(&path_src, &path_a, &path_b)?, dest: translate_path(&path_dest, &path_a, &path_b)?});
//            action_queue.push(SyncAction::CopyMeta {src: path_dest.clone(), dest: translate_path(&path_dest, &path_a, &path_b)?});
//            Ok(())
//        },
//        notify::DebouncedEvent::Rescan => {
//            //println!("rescan");
//            Ok(())
//        },
//        notify::DebouncedEvent::Error(_a,_b) => {
//            //println!("error");
//            Ok(())
//        }
//    }
//}

fn main() {
    let args: Vec<String> = env::args().collect();
    let path_a = PathBuf::from(&args[1]).canonicalize().unwrap();
    let path_b = PathBuf::from(&args[2]).canonicalize().unwrap();
    let interval: u64 = args[3].parse().unwrap();

    watch(&path_a, &path_b, interval);
}



