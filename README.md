# Twowaysync

This is a utility to keep two directories in sync. The intended use is to sync a local directory with one mounted from a file server.

## Usage

Simply start it with the command:
```
twowaysync /path/to/dirA /path/to/dirB interval
```

The first path is the local directory, and the second is the remote. The last argument "interval" is the time in seconds between syncs.

## How it works

The first time it's run on a pair of directories it will merge the contents, using the newest file from each one. It will then create an index file, called ".twoway.json" in each folder. This is used to catch file changes that happens while the program isn't running.

A sync means that both directories are scanned and compared with their indexes. Any change is then copied to the other directory. Whenever a change is copied, the indexes are regenerated.

If the remote folder disappears for some reason, the syncing will pause until the directory is available again.
