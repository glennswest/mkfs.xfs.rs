//! `xfs-admin`: `xfs_admin -U` and `-L` for an unmounted filesystem or
//! image, through [`mkfs_xfs::admin`].

use anyhow::{bail, Context, Result};
use clap::Parser;

use mkfs_xfs::admin;
use mkfs_xfs::device::FileDevice;

/// Change an unmounted XFS filesystem's UUID or label, as xfs_admin does.
#[derive(Parser, Debug)]
#[command(name = "xfs-admin", version, about)]
struct Cli {
    /// New UUID: a UUID, or `generate`, `nil`, or `restore` (back to the
    /// metadata UUID).
    #[arg(short = 'U', value_name = "uuid")]
    uuid: Option<String>,
    /// New label (at most 12 bytes); `--` clears it.
    #[arg(short = 'L', value_name = "label", allow_hyphen_values = true)]
    label: Option<String>,
    /// The device or image file.
    device: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.uuid.is_none() && cli.label.is_none() {
        bail!("nothing to do: give -U and/or -L");
    }
    let dev = FileDevice::open(&cli.device).await.with_context(|| format!("opening {}", cli.device))?;
    if let Some(label) = &cli.label {
        // check_label: "--" (or an empty quoted pair) clears the label.
        let label = match label.as_str() {
            "--" | "\"\"" | "''" => "",
            l => l,
        };
        admin::set_label(&dev, label).await?;
        println!("new label = \"{label}\"");
    }
    if let Some(u) = &cli.uuid {
        let uuid = match u.to_ascii_lowercase().as_str() {
            "generate" => *uuid::Uuid::new_v4().as_bytes(),
            "nil" => [0; 16],
            "restore" => {
                admin::restore_uuid(&dev).await?;
                return Ok(());
            }
            s => *uuid::Uuid::parse_str(s).with_context(|| format!("invalid UUID {s:?}"))?.as_bytes(),
        };
        admin::set_uuid(&dev, uuid).await?;
        println!("new UUID = {}", uuid::Uuid::from_bytes(uuid));
    }
    Ok(())
}
