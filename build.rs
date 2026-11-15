//! Builds native/ (our C bridge + chiaki-ng's libchiaki) with CMake against the
//! vcpkg dependencies in ./vcpkg_installed, then copies the DLLs next to the exe.
//!
//! Environment overrides:
//!   VS_INSTALL_DIR   Visual Studio root (default: found with vswhere)
//!   PS_PYTHON        python.exe with `protobuf` and `grpcio-tools` installed
//!   PS_SKIP_NATIVE=1 type-check the Rust code without building native/

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

const TRIPLET: &str = "x64-windows";

fn main() {
    println!("cargo:rerun-if-changed=native");
    println!("cargo:rerun-if-changed=vcpkg.json");
    println!("cargo:rerun-if-env-changed=PS_SKIP_NATIVE");
    println!("cargo:rerun-if-env-changed=PS_PYTHON");
    println!("cargo:rerun-if-env-changed=VS_INSTALL_DIR");
    println!("cargo:rerun-if-changed=third_party/chiaki-ng/lib");
    println!("cargo:rerun-if-changed=vcpkg_installed/vcpkg/status");

    if env::var("PS_SKIP_NATIVE").as_deref() == Ok("1") {
        return;
    }

    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let vcpkg_installed = root.join("vcpkg_installed");
    let deps = vcpkg_installed.join(TRIPLET);
    assert!(
        deps.join("include").exists(),
        "vcpkg dependencies missing. Run setup.ps1 (or `vcpkg install --triplet {TRIPLET} --x-install-root=vcpkg_installed`) first."
    );

    let vs = visual_studio_dir();
    let cmake_bin = vs.join(r"Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe");
    let ninja = vs.join(r"Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja\ninja.exe");
    let toolchain = vs.join(r"VC\vcpkg\scripts\buildsystems\vcpkg.cmake");
    // SAFETY: build scripts are single-threaded.
    unsafe { env::set_var("CMAKE", &cmake_bin) };

    let python = env::var("PS_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|_| root.join(r".venv\Scripts\python.exe"));
    assert!(
        python.is_file(),
        "Python environment missing. Run setup.ps1 or set PS_PYTHON."
    );
    let protoc = deps.join(r"tools\protobuf\protoc.exe");
    let pkgconf = deps.join(r"tools\pkgconf\pkgconf.exe");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    let dst = cmake::Config::new(root.join("native"))
        .generator("Ninja")
        .profile("Release")
        .define("CMAKE_MAKE_PROGRAM", fwd(&ninja))
        .define("CMAKE_TOOLCHAIN_FILE", fwd(&toolchain))
        .define("VCPKG_TARGET_TRIPLET", TRIPLET)
        .define("VCPKG_MANIFEST_MODE", "OFF")
        .define("VCPKG_INSTALLED_DIR", fwd(&vcpkg_installed))
        .define("CMAKE_PREFIX_PATH", fwd(&deps))
        .define("PYTHON_EXECUTABLE", fwd(&python))
        .define("PROTOC", fwd(&protoc))
        .define("PKG_CONFIG_EXECUTABLE", fwd(&pkgconf))
        .define("CMAKE_POLICY_VERSION_MINIMUM", "3.5")
        .build_target("psnative")
        .build();

    // build_target skips the install step, so pick the outputs from the build tree.
    let build_dir = dst.join("build");
    println!("cargo:rustc-link-search=native={}", build_dir.display());
    println!("cargo:rustc-link-lib=dylib=psnative");

    // Copy runtime DLLs next to the executable (target/<profile>/).
    let exe_dir = out_dir.ancestors().nth(3).unwrap().to_path_buf();
    copy_dll(&build_dir.join("psnative.dll"), &exe_dir);
    if let Ok(entries) = std::fs::read_dir(deps.join("bin")) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("dll")) {
                copy_dll(&p, &exe_dir);
            }
        }
    }
}

fn copy_dll(src: &Path, dir: &Path) {
    let dst = dir.join(src.file_name().unwrap());
    std::fs::copy(src, &dst).unwrap_or_else(|err| {
        panic!(
            "could not copy {} to {}: {err}",
            src.display(),
            dst.display()
        )
    });
}

/// CMake prefers forward slashes on Windows.
fn fwd(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

fn visual_studio_dir() -> PathBuf {
    if let Ok(dir) = env::var("VS_INSTALL_DIR") {
        return dir.into();
    }
    let vswhere = r"C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe";
    let out = Command::new(vswhere)
        .args([
            "-latest",
            "-products",
            "*",
            "-requires",
            "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
            "Microsoft.VisualStudio.Component.VC.CMake.Project",
            "-property",
            "installationPath",
        ])
        .output()
        .expect(
            "vswhere not found; install Visual Studio with the C++ workload or set VS_INSTALL_DIR",
        );
    let path = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    assert!(
        !path.is_empty(),
        "no Visual Studio with C++ tools found; set VS_INSTALL_DIR"
    );
    path.into()
}
