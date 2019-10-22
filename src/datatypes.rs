use std::path::PathBuf;
use std::cmp::Ordering;
use std::fs;
use filetime::FileTime;
use std::error::Error;
use std::fmt;
use serde::{Serialize, Deserialize};
use std::collections::HashMap;


#[derive(Clone, Debug, PartialEq)]
pub enum ChangeType {
    Newer,
    Older,
    NewOnly,
    RefOnly,
    Modified,
}

#[derive(Clone, Debug)]
pub struct DiffItem {
    pub diff: ChangeType,
    pub ftype: FileType,
    pub mtime: i64,
}

impl DiffItem {
    pub fn new(diff: ChangeType, ftype: FileType, mtime: i64) -> DiffItem {
        DiffItem { diff: diff, ftype: ftype, mtime: mtime }
    }
}

impl fmt::Display for ChangeType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ChangeType::Newer => write!(f,"Newer"),
            ChangeType::Older => write!(f,"Older"),
            ChangeType::NewOnly => write!(f,"Added"),
            ChangeType::RefOnly => write!(f,"Removed"),
            ChangeType::Modified => write!(f,"Modified"),
        }
    }
}

impl fmt::Display for DiffItem {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f,"{} {}, mtime: {}", self.diff, self.ftype, self.mtime)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub enum FileType {
    File,
    Dir,
    Link,
}

impl fmt::Display for FileType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            FileType::File => write!(f,"File"),
            FileType::Dir => write!(f,"Dir"),
            FileType::Link => write!(f,"Link"),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PathData {
    pub mtime: i64,
    pub perms: u32,
    pub size: u64,
    pub ftype: FileType,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct DirIndex {
    pub scantime: u64,
    pub root: PathBuf,
    pub contents: HashMap<PathBuf, PathData>,
}

impl PartialEq for PathData {
    fn eq(&self, other: &PathData) -> bool {
        self.mtime == other.mtime && self.perms == other.perms && self.size == other.size && self.ftype == other.ftype
    }
}

impl Eq for PathData {}




#[derive(Debug)]
pub enum SyncAction {
    CopyFile {src: PathBuf, dest: PathBuf},
    CopyDir {src: PathBuf, dest: PathBuf},
    CopyLink {src: PathBuf, dest: PathBuf},
    CopyMeta {src: PathBuf, dest: PathBuf},
    DeleteFile {dest: PathBuf},
    DeleteDir {dest: PathBuf},
}

pub trait Prio {
    fn prio(&self) -> usize;
}

impl Prio for SyncAction {
    fn prio(&self) -> usize {
        match self {
            &SyncAction::CopyFile {src: _, dest: _} => 2,
            &SyncAction::CopyDir {src: _, dest: _} => 1,
            &SyncAction::CopyLink {src: _, dest: _} => 4,
            &SyncAction::CopyMeta {src: _, dest: _} => 7,
            &SyncAction::DeleteFile {dest: _} => 5,
            &SyncAction::DeleteDir {dest: _} => 6,
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
            (&SyncAction::DeleteFile {dest: ref dest_a}, &SyncAction::DeleteFile {dest: ref dest_b})
            | (&SyncAction::DeleteDir {dest: ref dest_a}, &SyncAction::DeleteDir {dest: ref dest_b}) => (dest_a == dest_b),
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
            (&SyncAction::DeleteFile {dest: ref dest_a}, &SyncAction::DeleteFile {dest: ref dest_b})
            | (&SyncAction::DeleteDir {dest: ref dest_a}, &SyncAction::DeleteDir {dest: ref dest_b}) => dest_b.iter().count().cmp(&dest_a.iter().count()),
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
            SyncAction::DeleteFile {dest} => write!(f,"DeleteFile: {}",dest.display()),
            SyncAction::DeleteDir {dest} => write!(f,"DeleteDir: {}",dest.display()),
        }
    }
}


pub trait RunAction {
    fn run(&self) -> Result<(), Box<dyn Error>>;
}

impl RunAction for SyncAction {
    fn run(&self) -> Result<(), Box<dyn Error>> {
        match self {
            SyncAction::CopyFile {src, dest} => {
                if fs::metadata(&dest).is_ok() {
                    let mut perms = fs::metadata(&dest)?.permissions();
                    let readonly = perms.readonly();
                    if readonly {
                        perms.set_readonly(false);
                        fs::set_permissions(&dest, perms)?;
                    }
                }
                let _bytescopied = fs::copy(&src, &dest)?;
                Ok(())
            },
            SyncAction::CopyDir {src: _, dest} => {
                if !fs::metadata(&dest).is_ok() { 
                    fs::create_dir(&dest)?;
                }
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
                if fs::symlink_metadata(dest).is_ok() {
                    fs::remove_file(&dest)?;
                }
                std::os::unix::fs::symlink(target, dest)?;
                Ok(())
            },
            SyncAction::DeleteFile {dest} => {
                let mut perms = fs::metadata(&dest)?.permissions();
                let readonly = perms.readonly();
                if readonly {
                    perms.set_readonly(false);
                    fs::set_permissions(&dest, perms)?;
                }
                fs::remove_file(&dest)?;
                Ok(())
            },
            SyncAction::DeleteDir {dest} => {
                let mut perms = fs::metadata(&dest)?.permissions();
                let readonly = perms.readonly();
                if readonly {
                    perms.set_readonly(false);
                    fs::set_permissions(&dest, perms)?;
                }
                fs::remove_dir(&dest)?;
                Ok(())
            }
        }
    }
}