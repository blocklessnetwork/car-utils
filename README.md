# car-utils

The project is utils of `CAR` file which used in WASM runtime.

if you wanna lean about WASM runtime, please vist
"https://github.com/blocklessnetwork/runtime/".

## How to intsall.

Use cargo install to install the CLI:

```
cargo install car-utils
```

Note: car-utils installs to the cargo bin directory.

## How to use.

Execute the command `car-utils --help` to show the command help.

```
car-utils

Usage: car-utils <COMMAND>

Commands:
  pack    Pack files into a CAR
  unpack  Unpack files and directories from a CAR
  ls      List the car files
  roots   List root CIDs from a CAR
  cat     View cid content from a car file
  help    Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

### pack command

```
Pack files into a CAR

Usage: car-utils pack [OPTIONS] -o <OUTPUT> <SOURCE>

Arguments:
  <SOURCE>  The source file or directory to be packed

Options:
      --no-wrap    Wrap the file (applies to files only).
  -o <OUTPUT>      The car file to output.
  -h, --help       Print help
```

### unpack command

```
Unpack files and directories from a CAR

Usage: car-utils unpack [OPTIONS] <CAR>

Arguments:
  <CAR>  The car file to extract

Options:
  -o <OUTPUT>      Target directory to unpack car to.
  -h, --help       Print help
```

### ls command

```
List the car files

Usage: car-utils ls <CAR>

Arguments:
  <CAR>  the car file for list.

Options:
  -h, --help  Print help
```

#### roots command

```
List root CIDs from a CAR

Usage: car-utils roots <CAR>

Arguments:
  <CAR>  the car file for list.

Options:
  -h, --help  Print help
```

#### cat command

```
View cid content from a car file

Usage: car-utils cat -c <CID> <CAR>

Arguments:
  <CAR>  the car file to cat.

Options:
  -c <CID>      the cid of content to cat.
  -h, --help    Print help
```
