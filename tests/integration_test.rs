use proto_pdk_test_utils::*;
use std::process::Command;
use std::sync::OnceLock;
use std::{fs, path::Path};

fn ensure_wasm_built() {
    static BUILD_RESULT: OnceLock<Result<(), String>> = OnceLock::new();

    let result = BUILD_RESULT.get_or_init(|| {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));

        let output = Command::new("cargo")
            .args([
                "build",
                "--target",
                "wasm32-wasip1",
                "--release",
                "--features",
                "wasm",
                "--quiet",
            ])
            .env("CARGO_TARGET_DIR", manifest_dir.join("target"))
            .current_dir(manifest_dir)
            .output()
            .map_err(|error| format!("Failed to run cargo build for wasm target: {error}"))?;

        if !output.status.success() {
            return Err(format!(
                "Failed to build wasm plugin before tests (status: {}). stderr: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        let wasm_debug_dir = manifest_dir.join("target/wasm32-wasip1/debug");
        let wasm_release_dir = manifest_dir.join("target/wasm32-wasip1/release");
        let source_name = "proto_awscli.wasm";
        let expected_name = "proto-awscli.wasm";

        for dir in [&wasm_debug_dir, &wasm_release_dir] {
            let source = dir.join(source_name);
            let expected = dir.join(expected_name);

            if source.exists() {
                fs::copy(&source, &expected).map_err(|error| {
                    format!(
                        "Failed to create expected wasm file `{}` from `{}`: {}",
                        expected.display(),
                        source.display(),
                        error
                    )
                })?;
            }
        }

        Ok(())
    });

    if let Err(error) = result {
        panic!("{error}");
    }
}

mod awscli_tool {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn registers_tool_metadata() {
        ensure_wasm_built();
        let sandbox = create_empty_proto_sandbox();
        let plugin = sandbox.create_plugin("awscli-test").await;

        let output = plugin
            .register_tool(RegisterToolInput {
                id: Id::new("awscli-test").unwrap(),
            })
            .await;

        assert_eq!(output.name, "AWS CLI");
        assert_eq!(output.type_of, PluginType::CommandLine);
        assert!(output.minimum_proto_version.is_some());
        assert!(output.plugin_version.is_some());
        assert_eq!(output.self_upgrade_commands, vec!["upgrade"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn loads_versions_from_git_tags() {
        ensure_wasm_built();
        let sandbox = create_empty_proto_sandbox();
        let plugin = sandbox.create_plugin("awscli-test").await;

        let output = plugin.load_versions(LoadVersionsInput::default()).await;

        assert!(!output.versions.is_empty());

        // All versions should be v2
        for version in &output.versions {
            let v = version.to_string();
            assert!(
                v.starts_with("2."),
                "Expected version to start with '2.', got: {}",
                v
            );
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sets_latest_alias() {
        ensure_wasm_built();
        let sandbox = create_empty_proto_sandbox();
        let plugin = sandbox.create_plugin("awscli-test").await;

        let output = plugin.load_versions(LoadVersionsInput::default()).await;

        assert!(output.latest.is_some());
        assert!(output.aliases.contains_key("latest"));
        assert_eq!(output.aliases.get("latest"), output.latest.as_ref());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resolves_v2_alias() {
        ensure_wasm_built();
        let sandbox = create_empty_proto_sandbox();
        let plugin = sandbox.create_plugin("awscli-test").await;

        let output = plugin
            .resolve_version(ResolveVersionInput {
                initial: UnresolvedVersionSpec::Alias("v2".into()),
                ..Default::default()
            })
            .await;

        assert!(output.candidate.is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn downloads_linux_x64() {
        ensure_wasm_built();
        let sandbox = create_empty_proto_sandbox();
        let plugin = sandbox
            .create_plugin_with_config("awscli-test", |config| {
                config.host(HostOS::Linux, HostArch::X64);
            })
            .await;

        let output = plugin
            .download_prebuilt(DownloadPrebuiltInput {
                context: PluginContext {
                    version: VersionSpec::parse("2.22.0").unwrap(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .await;

        assert_eq!(
            output.download_url,
            "https://awscli.amazonaws.com/awscli-exe-linux-x86_64-2.22.0.zip"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn downloads_linux_arm64() {
        ensure_wasm_built();
        let sandbox = create_empty_proto_sandbox();
        let plugin = sandbox
            .create_plugin_with_config("awscli-test", |config| {
                config.host(HostOS::Linux, HostArch::Arm64);
            })
            .await;

        let output = plugin
            .download_prebuilt(DownloadPrebuiltInput {
                context: PluginContext {
                    version: VersionSpec::parse("2.22.0").unwrap(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .await;

        assert_eq!(
            output.download_url,
            "https://awscli.amazonaws.com/awscli-exe-linux-aarch64-2.22.0.zip"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn downloads_macos() {
        ensure_wasm_built();
        let sandbox = create_empty_proto_sandbox();
        let plugin = sandbox
            .create_plugin_with_config("awscli-test", |config| {
                config.host(HostOS::MacOS, HostArch::Arm64);
            })
            .await;

        let output = plugin
            .download_prebuilt(DownloadPrebuiltInput {
                context: PluginContext {
                    version: VersionSpec::parse("2.22.0").unwrap(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .await;

        assert_eq!(
            output.download_url,
            "https://awscli.amazonaws.com/AWSCLIV2-2.22.0.pkg"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn downloads_windows() {
        ensure_wasm_built();
        let sandbox = create_empty_proto_sandbox();
        let plugin = sandbox
            .create_plugin_with_config("awscli-test", |config| {
                config.host(HostOS::Windows, HostArch::X64);
            })
            .await;

        let output = plugin
            .download_prebuilt(DownloadPrebuiltInput {
                context: PluginContext {
                    version: VersionSpec::parse("2.22.0").unwrap(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .await;

        assert_eq!(
            output.download_url,
            "https://awscli.amazonaws.com/AWSCLIV2-2.22.0.msi"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn locates_linux_executables() {
        ensure_wasm_built();
        let sandbox = create_empty_proto_sandbox();
        let plugin = sandbox
            .create_plugin_with_config("awscli-test", |config| {
                config.host(HostOS::Linux, HostArch::X64);
            })
            .await;

        let output = plugin
            .locate_executables(LocateExecutablesInput {
                context: PluginContext {
                    version: VersionSpec::parse("2.22.0").unwrap(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .await;

        let aws_exe = output.exes.get("aws").unwrap();
        assert_eq!(aws_exe.exe_path, Some("bin/aws".into()));
        assert!(aws_exe.primary);

        let completer_exe = output.exes.get("aws_completer").unwrap();
        assert_eq!(completer_exe.exe_path, Some("bin/aws_completer".into()));
        assert!(!completer_exe.primary);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn locates_macos_executables() {
        ensure_wasm_built();
        let sandbox = create_empty_proto_sandbox();
        let plugin = sandbox
            .create_plugin_with_config("awscli-test", |config| {
                config.host(HostOS::MacOS, HostArch::Arm64);
            })
            .await;

        let output = plugin
            .locate_executables(LocateExecutablesInput {
                context: PluginContext {
                    version: VersionSpec::parse("2.22.0").unwrap(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .await;

        let aws_exe = output.exes.get("aws").unwrap();
        assert_eq!(aws_exe.exe_path, Some("aws".into()));
        assert!(aws_exe.primary);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn locates_windows_executables() {
        ensure_wasm_built();
        let sandbox = create_empty_proto_sandbox();
        let plugin = sandbox
            .create_plugin_with_config("awscli-test", |config| {
                config.host(HostOS::Windows, HostArch::X64);
            })
            .await;

        let output = plugin
            .locate_executables(LocateExecutablesInput {
                context: PluginContext {
                    version: VersionSpec::parse("2.22.0").unwrap(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .await;

        let aws_exe = output.exes.get("aws").unwrap();
        assert_eq!(aws_exe.exe_path, Some("Amazon/AWSCLIV2/aws.exe".into()));
        assert!(aws_exe.primary);
    }
}
