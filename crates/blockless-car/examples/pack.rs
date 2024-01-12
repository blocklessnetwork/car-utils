use blockless_car::utils::pack_files;

/// Cat the file in car file by file id
/// e.g. ```cargo run -p blockless-car --example pack <target-car-file>```
fn main() {
    let file_name = std::env::args().nth(1).expect("use directory as argument");
    let target = std::env::args()
        .nth(2)
        .expect("need the target file as argument");
    let file = std::fs::File::create(target).unwrap();
    pack_files(file_name, file, multicodec::Codec::Sha2_256, false).unwrap();
}
