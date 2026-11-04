//! Anvil region files (`r.X.Z.mca`): 32x32 chunks, zlib-compressed NBT in 4 KiB sectors.

use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, bail};
use meadow_nbt::Compound;

const SECTOR: usize = 4096;

/// Write a region file from already compressed chunk payloads, indexed by (cx & 31, cz & 31).
pub fn write_region(path: &Path, chunks: &[(i32, i32, Vec<u8>)]) -> Result<()> {
    let mut header = vec![0u8; SECTOR * 2];
    let mut body: Vec<u8> = Vec::with_capacity(chunks.iter().map(|c| c.2.len() + SECTOR).sum());
    let mut sector = 2usize;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as u32).unwrap_or(0);
    for (cx, cz, data) in chunks {
        let len = data.len() + 1; // + compression type byte
        let sectors = (len + 4).div_ceil(SECTOR);
        if sectors > 255 {
            bail!("chunk {cx},{cz} too large ({} bytes)", data.len());
        }
        let idx = ((cz & 31) * 32 + (cx & 31)) as usize;
        let loc = ((sector as u32) << 8) | sectors as u32;
        header[idx * 4..idx * 4 + 4].copy_from_slice(&loc.to_be_bytes());
        header[SECTOR + idx * 4..SECTOR + idx * 4 + 4].copy_from_slice(&now.to_be_bytes());
        body.extend_from_slice(&(len as u32).to_be_bytes());
        body.push(2); // zlib
        body.extend_from_slice(data);
        let pad = sectors * SECTOR - (len + 4);
        body.resize(body.len() + pad, 0);
        sector += sectors;
    }
    let tmp = path.with_extension("mca.tmp");
    {
        let mut f = std::io::BufWriter::new(std::fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?);
        f.write_all(&header)?;
        f.write_all(&body)?;
        f.flush()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Read every chunk in a region file (used for verification and the existing-map importer).
pub fn read_region(path: &Path) -> Result<Vec<(i32, i32, Compound)>> {
    let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    if data.len() < SECTOR * 2 {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for idx in 0..1024usize {
        let loc = u32::from_be_bytes(data[idx * 4..idx * 4 + 4].try_into().unwrap());
        if loc == 0 {
            continue;
        }
        let off = (loc >> 8) as usize * SECTOR;
        if off + 5 > data.len() {
            continue;
        }
        let len = u32::from_be_bytes(data[off..off + 4].try_into().unwrap()) as usize;
        let comp = data[off + 4];
        let payload = &data[off + 5..(off + 4 + len).min(data.len())];
        let raw = match comp {
            1 | 2 | 3 => meadow_nbt::decompress_any(payload)?,
            other => bail!("unsupported chunk compression {other}"),
        };
        let (_, root) = meadow_nbt::from_bytes(&raw)?;
        out.push(((idx % 32) as i32, (idx / 32) as i32, root));
    }
    Ok(out)
}
