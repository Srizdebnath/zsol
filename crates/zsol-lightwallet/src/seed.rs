//! BIP-39 seed generation and age encryption at rest (SRS FR-2.2, NF-4).
//! The identity file may live elsewhere (`ZSOL_AGE_IDENTITY`); by default it sits beside the
//! encrypted seed with mode 0600, the same model as zcash-devtool.

use std::io::{Read as _, Write as _};
use std::path::Path;

use anyhow::{Context as _, anyhow};
use bip0039::{Count, English, Mnemonic};
use secrecy::{ExposeSecret as _, SecretVec};

fn write_private(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("creating {}", path.display()))?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}

/// Loads the x25519 identity, generating it (0600) if absent.
pub fn load_or_create_identity(path: &Path) -> anyhow::Result<age::x25519::Identity> {
    use age::secrecy::ExposeSecret as _;
    if path.exists() {
        let text = std::fs::read_to_string(path)?;
        let line = text
            .lines()
            .find(|l| l.starts_with("AGE-SECRET-KEY-"))
            .ok_or_else(|| anyhow!("{} holds no AGE-SECRET-KEY line", path.display()))?;
        return line
            .parse::<age::x25519::Identity>()
            .map_err(|e| anyhow!("bad age identity: {e}"));
    }
    let id = age::x25519::Identity::generate();
    let body = format!(
        "# zsol wallet identity\n# public key: {}\n{}\n",
        id.to_public(),
        id.to_string().expose_secret()
    );
    write_private(path, body.as_bytes())?;
    Ok(id)
}

/// Generates a fresh 24-word mnemonic, stores it age-encrypted, returns the 64-byte seed.
pub fn create_seed(seed_path: &Path, id: &age::x25519::Identity) -> anyhow::Result<SecretVec<u8>> {
    let mnemonic = <Mnemonic<English>>::generate(Count::Words24);
    let recipient = id.to_public();
    let encryptor =
        age::Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient))?;
    let mut ciphertext = vec![];
    let mut w = encryptor.wrap_output(age::armor::ArmoredWriter::wrap_output(
        &mut ciphertext,
        age::armor::Format::AsciiArmor,
    )?)?;
    w.write_all(mnemonic.phrase().as_bytes())?;
    w.finish().and_then(|a| a.finish())?;
    write_private(seed_path, &ciphertext)?;
    Ok(SecretVec::new(mnemonic.to_seed("").to_vec()))
}

/// Decrypts the stored mnemonic and derives the seed.
pub fn load_seed(seed_path: &Path, id: &age::x25519::Identity) -> anyhow::Result<SecretVec<u8>> {
    let ciphertext =
        std::fs::read(seed_path).with_context(|| format!("reading {}", seed_path.display()))?;
    let decryptor = age::Decryptor::new(age::armor::ArmoredReader::new(&ciphertext[..]))?;
    let mut buf = vec![];
    let ret = decryptor
        .decrypt(std::iter::once(id as &dyn age::Identity))?
        .read_to_end(&mut buf);
    let phrase = SecretVec::new(buf);
    ret?;
    let text = std::str::from_utf8(phrase.expose_secret()).context("mnemonic utf-8")?;
    let mnemonic = <Mnemonic<English>>::from_phrase(text.trim())?;
    Ok(SecretVec::new(mnemonic.to_seed("").to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_roundtrips_through_age_and_files_are_private() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!("zsol-seed-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let id = load_or_create_identity(&dir.join("identity.txt")).unwrap();
        let s1 = create_seed(&dir.join("seed.age"), &id).unwrap();
        let id2 = load_or_create_identity(&dir.join("identity.txt")).unwrap();
        let s2 = load_seed(&dir.join("seed.age"), &id2).unwrap();
        assert_eq!(s1.expose_secret(), s2.expose_secret());
        for f in ["identity.txt", "seed.age"] {
            let mode = std::fs::metadata(dir.join(f)).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{f}");
        }
        let other = age::x25519::Identity::generate();
        assert!(load_seed(&dir.join("seed.age"), &other).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
