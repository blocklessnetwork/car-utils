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

execute the command `car-utils --help` to show the command help.

```
Usage: car-utils <COMMAND>

Commands:
  ar    Archive local file system to a car file
  cat   View cid content from a car file
  ls    List the car files
  cid   List the car cid
  ex    Extract the car files
  help  Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

###  ar subcommand

archive the local directory to the car file.

```
Archive local file system to a car file

Usage: car-utils ar -c <CAR> -s <SOURCE>

Options:
  -c <CAR>         the car file for archive.
  -s <SOURCE>      the source directory to be archived.
  -h, --help       Print help
```

###  ls subcommand

list file structures in the car file.

```
List the car files

Usage: car-utils ls <CAR>

Arguments:
  <CAR>  the car file for list.

Options:
  -h, --help  Print help
```

####  cid subcommand

list file cids in the car file.

```
List the car cid

Usage: car-utils cid <CAR>

Arguments:
  <CAR>  the car file for list.

Options:
  -h, --help  Print help
```

### ex subcommand

extract the files in the car file to the target directory.

```
Extract the car files

Usage: car-utils ex [OPTIONS] -c <CAR>

Options:
  -c <CAR>         The car file to extract
  -t <TARGET>      Target directory to extract to
  -h, --help       Print help
```

####  cat subcommand

cat cid content from a car file.

```
View cid content from a car file

Usage: car-utils cat -c <CID> <CAR>

Arguments:
  <CAR>  the car file to cat.

Options:
  -c <CID>      the cid of content to cat.
  -h, --help    Print help
```
