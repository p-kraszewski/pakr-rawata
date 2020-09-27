use std::{env, path::PathBuf};

fn main() {
    if cfg!(target_os = "freebsd") {
        println!("cargo:rustc-link-lib=cam");

        // The bindgen::Builder is the main entry point
        // to bindgen, and lets you build up options for
        // the resulting bindings.
        let bindings_libcam = bindgen::Builder::default()
            .opaque_type("nvme_registers")
            .opaque_type("cap_lo_register")
            .opaque_type("cap_hi_register")
            .opaque_type("csts_register")
            .opaque_type("aqa_register")
            .opaque_type("cc_register")
            .opaque_type("max_align_t")
            .header("c-bindings/freebsd_ata.h")
            .generate_comments(true)
            .generate()
            .expect("Unable to generate bindings");

        let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
        bindings_libcam
            .write_to_file(out_path.join("libcam-bind.rs"))
            .expect("Couldn't write bindings!");
    }
}
