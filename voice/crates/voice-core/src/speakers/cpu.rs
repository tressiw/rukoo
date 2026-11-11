//! The processor features the portable NeMo-Speech.cpp builds need: on Windows an AVX2 baseline
//! with FMA, F16C and BMI2, on macOS an ARMv8.2-A baseline with NEON, dot-product and FP16
//! arithmetic. The app checks them before it starts a worker, and the worker again before it
//! loads the runtime.

use serde::Serialize;

/// The features the build for this platform needs, in the order they are reported.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const REQUIRED: [&str; 3] = ["neon", "dotprod", "fp16"];
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
const REQUIRED: [&str; 4] = ["avx2", "fma", "f16c", "bmi2"];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CpuFeatures {
    /// The Windows build's baseline.
    pub avx2: bool,
    pub fma: bool,
    pub f16c: bool,
    pub bmi2: bool,
    /// The macOS build's baseline.
    pub neon: bool,
    pub dotprod: bool,
    pub fp16: bool,
}

/// This processor's features, as the build for this platform needs them. A feature of another
/// platform is false; [`CpuFeatures::missing`] only asks for the ones this build uses.
pub fn cpu_features() -> CpuFeatures {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        CpuFeatures {
            neon: true,
            dotprod: arm_feature("hw.optional.arm.FEAT_DotProd"),
            fp16: arm_feature("hw.optional.arm.FEAT_FP16"),
            ..CpuFeatures::default()
        }
    }
    #[cfg(target_arch = "x86_64")]
    {
        CpuFeatures {
            avx2: std::arch::is_x86_feature_detected!("avx2"),
            fma: std::arch::is_x86_feature_detected!("fma"),
            f16c: std::arch::is_x86_feature_detected!("f16c"),
            bmi2: std::arch::is_x86_feature_detected!("bmi2"),
            ..CpuFeatures::default()
        }
    }
    #[cfg(not(any(
        target_arch = "x86_64",
        all(target_os = "macos", target_arch = "aarch64")
    )))]
    {
        CpuFeatures::default()
    }
}

/// An Apple Silicon CPU feature as `sysctl` reports it. A key this macOS doesn't know is a Mac
/// that can't answer rather than one without the feature: every Apple Silicon Mac has both.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn arm_feature(name: &str) -> bool {
    let name = std::ffi::CString::new(name).expect("a sysctl name has no NUL");
    let mut value: i32 = 0;
    let mut size = std::mem::size_of::<i32>();
    let status = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            (&mut value as *mut i32).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    status != 0 || value != 0
}

impl CpuFeatures {
    /// Whether this processor has the feature `name`, as [`CpuFeatures::missing`] names them.
    fn has(&self, name: &str) -> bool {
        match name {
            "avx2" => self.avx2,
            "fma" => self.fma,
            "f16c" => self.f16c,
            "bmi2" => self.bmi2,
            "neon" => self.neon,
            "dotprod" => self.dotprod,
            "fp16" => self.fp16,
            _ => false,
        }
    }

    /// The names of the missing features among `required`, in the order given. It takes the list,
    /// so the baseline of either build can be checked on any machine.
    pub fn missing_among(&self, required: &[&'static str]) -> Vec<&'static str> {
        required
            .iter()
            .copied()
            .filter(|name| !self.has(name))
            .collect()
    }

    /// The names of the missing features of this platform's build, in a fixed order; empty when
    /// the runtime can run.
    pub fn missing(&self) -> Vec<&'static str> {
        self.missing_among(&REQUIRED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: CpuFeatures = CpuFeatures {
        avx2: true,
        fma: true,
        f16c: true,
        bmi2: true,
        neon: true,
        dotprod: true,
        fp16: true,
    };

    #[test]
    fn missing_features_are_the_platforms_baseline() {
        assert_eq!(CpuFeatures::default().missing(), REQUIRED);
        assert!(ALL.missing().is_empty());
    }

    /// The x86-64 runtime's baseline, whichever platform runs this test.
    const X86: [&str; 4] = ["avx2", "fma", "f16c", "bmi2"];
    /// The arm64 runtime's baseline.
    const ARM: [&str; 3] = ["neon", "dotprod", "fp16"];

    #[test]
    fn the_x86_baseline_is_named_in_order() {
        let complete = CpuFeatures {
            avx2: true,
            fma: true,
            f16c: true,
            bmi2: true,
            ..CpuFeatures::default()
        };
        assert!(complete.missing_among(&X86).is_empty());
        // Everything but AVX2 and FMA.
        let old = CpuFeatures {
            f16c: true,
            bmi2: true,
            ..CpuFeatures::default()
        };
        assert_eq!(old.missing_among(&X86), ["avx2", "fma"]);
        assert_eq!(CpuFeatures::default().missing_among(&X86), X86);
    }

    #[test]
    fn the_arm_baseline_is_named_in_order() {
        let complete = CpuFeatures {
            neon: true,
            dotprod: true,
            fp16: true,
            ..CpuFeatures::default()
        };
        assert!(complete.missing_among(&ARM).is_empty());
        let no_dotprod = CpuFeatures {
            neon: true,
            fp16: true,
            ..CpuFeatures::default()
        };
        assert_eq!(no_dotprod.missing_among(&ARM), ["dotprod"]);
        assert_eq!(CpuFeatures::default().missing_among(&ARM), ARM);
    }

    #[test]
    fn the_features_of_both_builds_are_serialized() {
        let value = serde_json::to_value(ALL).unwrap();
        for name in ["avx2", "fma", "f16c", "bmi2", "neon", "dotprod", "fp16"] {
            assert_eq!(value[name], serde_json::json!(true), "{name}");
        }
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn detection_agrees_with_the_standard_library() {
        let detected = cpu_features();
        assert_eq!(detected.avx2, std::arch::is_x86_feature_detected!("avx2"));
        assert_eq!(detected.bmi2, std::arch::is_x86_feature_detected!("bmi2"));
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    fn an_apple_silicon_mac_has_its_baseline() {
        let detected = cpu_features();
        assert!(detected.neon);
        assert_eq!(
            detected.dotprod,
            arm_feature("hw.optional.arm.FEAT_DotProd")
        );
        assert_eq!(detected.fp16, arm_feature("hw.optional.arm.FEAT_FP16"));
        assert!(detected.missing().is_empty());
    }
}
