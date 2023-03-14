# car-utils

The project is utils of CAR file which used in WASM runtime.

if you wanna lean about WASM runtime, please vist "https://github.com/blocklessnetwork/runtime/".

## How to intsall.

use cargo install to install the command
```
cargo install car-utils
```
car-utils install in the cargo bin directory.

## How to use.
execute the command `car-utils -help` to show the command help.
```
Usage: car-utils [COMMAND]

Commands:
  ar    archive local file system to a car file.
  ls    list the car files
  ex    extract the car files
  help  Print this message or the help of the given subcommand(s)

Options:
  -h, --help  Print help
```

###  ar subcommand
archive the local directory to the car file.
```
archive local file system to a car file.

Usage: car-utils ar -c <car> -s <source>

Options:
  -c <car>         the car file for archive
  -s <source>      the source directory to archived
  -h, --help       Print help
```

###  ls subcommand
list file structures in the car file.
```
list the car files

Usage: car-utils ls -c <car>

Options:
  -c <car>      the car file for list
  -h, --help    Print help
```

####  cid subcommand
list file cids in the car file.
```
list the car cid

Usage: car-utils cid -c <car>

Options:
  -c <car>      the car file for list.
  -h, --help    Print help
```
### ex subcommand
extract the files in the car file to the target directory.
```
Usage: car-utils ex [OPTIONS] -c <car>

Options:
  -c <car>         the car file for extract
  -t <target>      the target directory to extract
  -h, --help       Print help
```
