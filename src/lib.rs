use extism_pdk::*;
use proto_pdk::*;
use std::collections::HashMap;

#[host_fn]
extern "ExtismHost" {
    fn download_file(input: Json<DownloadFileInput>) -> Json<DownloadFileOutput>;
    fn exec_command(input: Json<ExecCommandInput>) -> Json<ExecCommandOutput>;
}

static NAME: &str = "AWS CLI";

/// Format a host path for native Windows tools.
///
/// Under WASI, `VirtualPath::to_real_path()` keeps `C:\...` prefixes as a single
/// path component and later joins with `/`, producing mixed separators that
/// `msiexec` cannot open (Windows Installer error 1619).
fn windows_host_path(path: &str) -> String {
    path.replace('\\', "/").replace('/', "\\")
}

fn windows_join(dir: &str, name: &str) -> String {
    format!(
        "{}\\{}",
        windows_host_path(dir).trim_end_matches('\\'),
        name
    )
}

#[plugin_fn]
pub fn register_tool(Json(_): Json<RegisterToolInput>) -> FnResult<Json<RegisterToolOutput>> {
    Ok(Json(RegisterToolOutput {
        name: NAME.into(),
        type_of: PluginType::CommandLine,
        minimum_proto_version: Some(Version::new(0, 61, 0)),
        plugin_version: Version::parse(env!("CARGO_PKG_VERSION")).ok(),
        self_upgrade_commands: vec!["upgrade".into()],
        ..RegisterToolOutput::default()
    }))
}

#[plugin_fn]
pub fn load_versions(Json(_): Json<LoadVersionsInput>) -> FnResult<Json<LoadVersionsOutput>> {
    let tags = load_git_tags("https://github.com/aws/aws-cli")?
        .into_iter()
        .filter(|tag| {
            tag.starts_with("2.")
                && !tag.contains("dev")
                && !tag.contains("alpha")
                && !tag.contains("beta")
                && !tag.contains("rc")
                && tag.chars().all(|c| c.is_ascii_digit() || c == '.')
        })
        .collect::<Vec<_>>();

    Ok(Json(LoadVersionsOutput::from(tags)?))
}

#[plugin_fn]
pub fn resolve_version(
    Json(input): Json<ResolveVersionInput>,
) -> FnResult<Json<ResolveVersionOutput>> {
    let mut output = ResolveVersionOutput::default();

    // Map "v2" alias to latest v2 range
    let initial = input.initial.to_string();

    if initial == "v2" || initial == "2" {
        output.candidate = Some(UnresolvedVersionSpec::parse(">=2.0.0")?);
    }

    Ok(Json(output))
}

#[plugin_fn]
pub fn download_prebuilt(
    Json(input): Json<DownloadPrebuiltInput>,
) -> FnResult<Json<DownloadPrebuiltOutput>> {
    let env = get_host_environment()?;
    let version = &input.context.version;

    if version.is_canary() {
        return Err(plugin_err!(PluginError::UnsupportedCanary {
            tool: NAME.into()
        }));
    }

    check_supported_os_and_arch(
        NAME,
        env,
        permutations![
            HostOS::Linux => [HostArch::X64, HostArch::Arm64],
            HostOS::MacOS => [HostArch::X64, HostArch::Arm64],
            HostOS::Windows => [HostArch::X64],
        ],
    )?;

    let version_str = version.to_string();

    let download_url = match env.os {
        HostOS::Linux => {
            let arch = match env.arch {
                HostArch::Arm64 => "aarch64",
                _ => "x86_64",
            };
            format!("https://awscli.amazonaws.com/awscli-exe-linux-{arch}-{version_str}.zip")
        }
        HostOS::MacOS => {
            format!("https://awscli.amazonaws.com/AWSCLIV2-{version_str}.pkg")
        }
        HostOS::Windows => {
            format!("https://awscli.amazonaws.com/AWSCLIV2-{version_str}.msi")
        }
        _ => unreachable!(),
    };

    let download_name = match env.os {
        HostOS::Linux => {
            let arch = match env.arch {
                HostArch::Arm64 => "aarch64",
                _ => "x86_64",
            };
            Some(format!("awscli-exe-linux-{arch}-{version_str}.zip"))
        }
        HostOS::MacOS => Some(format!("AWSCLIV2-{version_str}.pkg")),
        HostOS::Windows => Some(format!("AWSCLIV2-{version_str}.msi")),
        _ => None,
    };

    Ok(Json(DownloadPrebuiltOutput {
        download_url,
        download_name,
        ..DownloadPrebuiltOutput::default()
    }))
}

#[plugin_fn]
pub fn native_install(
    Json(input): Json<NativeInstallInput>,
) -> FnResult<Json<NativeInstallOutput>> {
    let env = get_host_environment()?;
    let install_dir_real = input.install_dir.to_real_path()?.ok_or_else(|| {
        anyhow!(
            "Failed to convert install_dir to a real path: {}",
            input.install_dir
        )
    })?;
    let install_dir_str = install_dir_real.to_string_lossy().to_string();
    let version = &input.context.version;
    let version_str = version.to_string();

    let temp_dir = &input.context.temp_dir;
    let temp_dir_real = temp_dir
        .to_real_path()?
        .ok_or_else(|| anyhow!("Failed to convert temp_dir to a real path: {}", temp_dir))?;
    let temp_dir_str = temp_dir_real.to_string_lossy().to_string();

    match env.os {
        HostOS::Linux => {
            let arch = match env.arch {
                HostArch::Arm64 => "aarch64",
                _ => "x86_64",
            };

            let zip_name = format!("awscli-exe-linux-{arch}-{version_str}.zip");
            let zip_url = format!("https://awscli.amazonaws.com/{zip_name}");
            let zip_virt = VirtualPath::new(temp_dir.join(&zip_name));
            let zip_path = temp_dir_real.join(&zip_name);
            let zip_path_str = zip_path.to_string_lossy().to_string();

            debug!("Downloading AWS CLI from <url>{}</url>", zip_url);

            download_from_url(&zip_url, &zip_virt)?;

            debug!("Extracting AWS CLI archive");

            let unzip_output = exec_captured("unzip", ["-o", &zip_path_str, "-d", &temp_dir_str])?;

            if unzip_output.exit_code != 0 {
                return Ok(Json(NativeInstallOutput {
                    installed: false,
                    error: Some(format!(
                        "Failed to unzip AWS CLI archive: {}",
                        unzip_output.stderr
                    )),
                    ..NativeInstallOutput::default()
                }));
            }

            let installer_path = format!("{}/aws/install", temp_dir_str);
            let bin_dir = format!("{}/bin", install_dir_str);

            debug!(
                "Running AWS CLI installer to <path>{}</path>",
                install_dir_str.clone()
            );

            let install_output = exec(ExecCommandInput {
                command: installer_path,
                args: vec![
                    "--install-dir".into(),
                    install_dir_str.clone(),
                    "--bin-dir".into(),
                    bin_dir,
                    "--update".into(),
                ],
                set_executable: true,
                stream: true,
                ..ExecCommandInput::default()
            })?;

            if install_output.exit_code != 0 {
                return Ok(Json(NativeInstallOutput {
                    installed: false,
                    error: Some(format!(
                        "AWS CLI installer failed: {}",
                        install_output.stderr
                    )),
                    ..NativeInstallOutput::default()
                }));
            }
        }
        HostOS::MacOS => {
            let pkg_name = format!("AWSCLIV2-{version_str}.pkg");
            let pkg_url = format!("https://awscli.amazonaws.com/{pkg_name}");
            let pkg_virt = VirtualPath::new(temp_dir.join(&pkg_name));
            let pkg_path = temp_dir_real.join(&pkg_name);
            let pkg_path_str = pkg_path.to_string_lossy().to_string();

            debug!("Downloading AWS CLI from <url>{}</url>", pkg_url);

            download_from_url(&pkg_url, &pkg_virt)?;

            let expanded_dir = format!("{}/aws-cli-expanded", temp_dir_str);

            debug!("Expanding AWS CLI package");

            let expand_output =
                exec_captured("pkgutil", ["--expand-full", &pkg_path_str, &expanded_dir])?;

            if expand_output.exit_code != 0 {
                return Ok(Json(NativeInstallOutput {
                    installed: false,
                    error: Some(format!(
                        "Failed to expand AWS CLI package: {}",
                        expand_output.stderr
                    )),
                    ..NativeInstallOutput::default()
                }));
            }

            let payload_dir = format!("{}/aws-cli.pkg/Payload/aws-cli", expanded_dir);

            debug!(
                "Copying AWS CLI to <path>{}</path>",
                install_dir_str.clone()
            );

            let copy_output = exec_captured(
                "cp",
                [
                    "-R",
                    &format!("{}/.", payload_dir),
                    &install_dir_str.clone(),
                ],
            )?;

            if copy_output.exit_code != 0 {
                return Ok(Json(NativeInstallOutput {
                    installed: false,
                    error: Some(format!(
                        "Failed to copy AWS CLI files: {}",
                        copy_output.stderr
                    )),
                    ..NativeInstallOutput::default()
                }));
            }
        }
        HostOS::Windows => {
            let msi_name = format!("AWSCLIV2-{version_str}.msi");
            let msi_url = format!("https://awscli.amazonaws.com/{msi_name}");
            let msi_virt = VirtualPath::new(temp_dir.join(&msi_name));
            // Guest `Path::join` under WASI treats Windows host prefixes as a single
            // component and appends with `/`, which msiexec rejects (error 1619).
            // Normalize to backslash paths for native Windows tools.
            let msi_path_str = windows_join(&temp_dir_str, &msi_name);
            let install_dir_win = windows_host_path(&install_dir_str);
            let msi_log_path = windows_join(&temp_dir_str, "awscli-msi.log");

            debug!("Downloading AWS CLI from <url>{}</url>", msi_url);

            download_from_url(&msi_url, &msi_virt)?;

            // Use administrative extract (`/a`) into proto's install dir instead of
            // `/i` + INSTALLDIR. The all-users MSI needs elevation and commonly
            // fails with exit 1603 in CI / non-system install roots.
            debug!("Extracting AWS CLI MSI to <path>{}</path>", install_dir_win);

            let install_output = exec_captured(
                "msiexec",
                [
                    "/a",
                    &msi_path_str,
                    "/qn",
                    "/norestart",
                    &format!("TARGETDIR={install_dir_win}"),
                    "/L*v",
                    &msi_log_path,
                ],
            )?;

            if install_output.exit_code != 0 {
                let log_tail = exec_captured(
                    "powershell",
                    [
                        "-NoProfile",
                        "-Command",
                        &format!(
                            "if (Test-Path -LiteralPath '{}') {{ Get-Content -LiteralPath '{}' -Tail 40 | Out-String }}",
                            msi_log_path.replace('\'', "''"),
                            msi_log_path.replace('\'', "''"),
                        ),
                    ],
                )
                .map(|output| output.stdout)
                .unwrap_or_default();

                return Ok(Json(NativeInstallOutput {
                    installed: false,
                    error: Some(format!(
                        "AWS CLI MSI extraction failed (exit {}): {}\n{}\n{}",
                        install_output.exit_code,
                        install_output.stderr,
                        install_output.stdout,
                        log_tail
                    )),
                    ..NativeInstallOutput::default()
                }));
            }
        }
        _ => {
            return Ok(Json(NativeInstallOutput {
                installed: false,
                error: Some(format!("Unsupported operating system: {}", env.os)),
                ..NativeInstallOutput::default()
            }));
        }
    }

    Ok(Json(NativeInstallOutput {
        installed: true,
        ..NativeInstallOutput::default()
    }))
}

#[plugin_fn]
pub fn native_uninstall(
    Json(input): Json<NativeUninstallInput>,
) -> FnResult<Json<NativeUninstallOutput>> {
    // Proto removes the install directory. Windows uses MSI extract (`/a`), so
    // there is no registered product to uninstall via `msiexec /x`.
    debug!(
        "Removing AWS CLI from <path>{}</path>",
        input.uninstall_dir.to_string()
    );

    Ok(Json(NativeUninstallOutput {
        uninstalled: true,
        ..NativeUninstallOutput::default()
    }))
}

#[plugin_fn]
pub fn locate_executables(
    Json(_): Json<LocateExecutablesInput>,
) -> FnResult<Json<LocateExecutablesOutput>> {
    let env = get_host_environment()?;

    let (exe_path, completer_path) = match env.os {
        HostOS::Linux => ("bin/aws".to_string(), "bin/aws_completer".to_string()),
        HostOS::MacOS => ("aws".to_string(), "aws_completer".to_string()),
        // `msiexec /a` extracts under Amazon/AWSCLIV2 on Windows.
        HostOS::Windows => (
            "Amazon/AWSCLIV2/aws.exe".to_string(),
            "Amazon/AWSCLIV2/aws_completer.exe".to_string(),
        ),
        _ => ("aws".to_string(), "aws_completer".to_string()),
    };

    Ok(Json(LocateExecutablesOutput {
        exes: HashMap::from_iter([
            ("aws".into(), ExecutableConfig::new_primary(&exe_path)),
            (
                "aws_completer".into(),
                ExecutableConfig::new(&completer_path),
            ),
        ]),
        ..LocateExecutablesOutput::default()
    }))
}
