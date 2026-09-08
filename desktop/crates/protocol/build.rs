fn main() {
    let files = [
        "../../../proto/photosync/v1/sync.proto",
        "../../../proto/photosync/v1/pairing.proto",
    ];
    for file in files {
        println!("cargo:rerun-if-changed={file}");
    }

    prost_build::compile_protos(&files, &["../../../proto"]).expect("protobuf schema is valid");
}
