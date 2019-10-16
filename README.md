# Twowaysync

This is a utility to keep two directories in sync. The intended use is to sync a local directory with one mounted from a file server.

## Usage

Run it with the help flag, -h and it will display usage information:
```sh
TwoWaySync 0.1.1
Henrik Enquist <henrik.enquist@gmail.com>
Sync two directories

USAGE:
    twowaysync [FLAGS] [OPTIONS] <dir_a> <dir_b>

FLAGS:
    -c               Compare and show diff (default)
    -h, --help       Prints help information
    -s               Do a single sync only
    -V, --version    Prints version information

OPTIONS:
    -w, --watch <interval>    Interval in seconds to watch for changes

ARGS:
    <dir_a>    First directory
    <dir_b>    Second directory
```

Option | Explanation
--- | ---
-c | Compare the directories and print a diff, no files are modified
-h | Prints help
-s | Compare the two directories and sync their contents
-w \<interval\> | Watch both directories for changes every \<interval\> seconds and sync them


Example 

```
twowaysync -w 10 /path/to/local/dir /path/to/remote/dir 
```

This watches the two given directories for changes and syncs them every 10 seconds. The two paths have equal priority so local and remote can be swapped. 


## How it works

The first time it's run on a pair of directories it will merge the contents, using the newest file from each one. It will then create an index file, called ".twoway.json" in each folder. This is used to catch file changes that happens while the program isn't running.

A sync means that both directories are scanned and compared with their indexes. Any change is then copied to the other directory. Whenever a change is copied, the indexes are regenerated.

If one of the folders becomes unreadable the syncing will pause until the directory is available again.
