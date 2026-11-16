//! SHA-256 y verificación de firmas ECDSA P-256 con CNG (`bcrypt.dll`),
//! resuelto en tiempo de ejecución (hito U, `plan-update.md` U2): `wm.exe`
//! no importa `bcrypt.dll` (el guardián y el core no la cargan) ni suma
//! crates de criptografía.

use std::ffi::c_void;
use std::io::Read;
use std::path::Path;

use windows::core::{PCSTR, PCWSTR, w};

type Handle = *mut c_void;
type OpenAlg = unsafe extern "system" fn(*mut Handle, PCWSTR, PCWSTR, u32) -> i32;
type CloseAlg = unsafe extern "system" fn(Handle, u32) -> i32;
type ImportKeyPair =
    unsafe extern "system" fn(Handle, Handle, PCWSTR, *mut Handle, *const u8, u32, u32) -> i32;
type DestroyKey = unsafe extern "system" fn(Handle) -> i32;
type VerifySig =
    unsafe extern "system" fn(Handle, *const c_void, *const u8, u32, *const u8, u32, u32) -> i32;
type CreateHash =
    unsafe extern "system" fn(Handle, *mut Handle, *mut u8, u32, *const u8, u32, u32) -> i32;
type HashData = unsafe extern "system" fn(Handle, *const u8, u32, u32) -> i32;
type FinishHash = unsafe extern "system" fn(Handle, *mut u8, u32, u32) -> i32;
type DestroyHash = unsafe extern "system" fn(Handle) -> i32;

type Raw = unsafe extern "system" fn() -> isize;

/// Puntero de `GetProcAddress` a la firma real `T` (otro puntero a función).
///
/// # Safety
/// `T` tiene que ser la firma verdadera de la función.
unsafe fn cast<T: Copy>(f: Raw) -> T {
    debug_assert_eq!(std::mem::size_of::<T>(), std::mem::size_of::<Raw>());
    // SAFETY: mismo tamaño (punteros a función); la firma la garantiza quien llama.
    unsafe { std::mem::transmute_copy(&f) }
}

struct Cng {
    open: OpenAlg,
    close: CloseAlg,
    import: ImportKeyPair,
    destroy_key: DestroyKey,
    verify: VerifySig,
    create_hash: CreateHash,
    hash_data: HashData,
    finish_hash: FinishHash,
    destroy_hash: DestroyHash,
}

fn cng() -> Option<Cng> {
    let f =
        |name: &[u8]| crate::dynlib::proc(w!("bcrypt.dll"), PCSTR(name.as_ptr())).ok().flatten();
    // SAFETY: firmas de bcrypt.h; cada puntero viene de la función de ese nombre.
    unsafe {
        Some(Cng {
            open: cast(f(b"BCryptOpenAlgorithmProvider\0")?),
            close: cast(f(b"BCryptCloseAlgorithmProvider\0")?),
            import: cast(f(b"BCryptImportKeyPair\0")?),
            destroy_key: cast(f(b"BCryptDestroyKey\0")?),
            verify: cast(f(b"BCryptVerifySignature\0")?),
            create_hash: cast(f(b"BCryptCreateHash\0")?),
            hash_data: cast(f(b"BCryptHashData\0")?),
            finish_hash: cast(f(b"BCryptFinishHash\0")?),
            destroy_hash: cast(f(b"BCryptDestroyHash\0")?),
        })
    }
}

/// SHA-256 incremental.
pub struct Sha256 {
    cng: Cng,
    alg: Handle,
    hash: Handle,
}

impl Sha256 {
    pub fn new() -> Option<Self> {
        let cng = cng()?;
        let mut alg: Handle = std::ptr::null_mut();
        // SAFETY: punteros a variables locales; nombre de algoritmo constante.
        if unsafe { (cng.open)(&mut alg, w!("SHA256"), PCWSTR::null(), 0) } != 0 {
            return None;
        }
        let mut hash: Handle = std::ptr::null_mut();
        // SAFETY: proveedor recién abierto; CNG reserva el objeto (Win 7+).
        if unsafe {
            (cng.create_hash)(alg, &mut hash, std::ptr::null_mut(), 0, std::ptr::null(), 0, 0)
        } != 0
        {
            // SAFETY: proveedor propio.
            unsafe { (cng.close)(alg, 0) };
            return None;
        }
        Some(Self { cng, alg, hash })
    }

    pub fn update(&mut self, data: &[u8]) {
        for chunk in data.chunks(u32::MAX as usize) {
            // SAFETY: slice válido; hash propio.
            unsafe { (self.cng.hash_data)(self.hash, chunk.as_ptr(), chunk.len() as u32, 0) };
        }
    }

    pub fn finish(self) -> [u8; 32] {
        let mut out = [0u8; 32];
        // SAFETY: buffer de 32 bytes (SHA-256); hash propio.
        unsafe { (self.cng.finish_hash)(self.hash, out.as_mut_ptr(), 32, 0) };
        out
    }
}

impl Drop for Sha256 {
    fn drop(&mut self) {
        // SAFETY: objetos propios, liberados una sola vez.
        unsafe {
            (self.cng.destroy_hash)(self.hash);
            (self.cng.close)(self.alg, 0);
        }
    }
}

pub fn sha256(data: &[u8]) -> Option<[u8; 32]> {
    let mut h = Sha256::new()?;
    h.update(data);
    Some(h.finish())
}

/// SHA-256 de un archivo (y su tamaño).
pub fn sha256_file(path: &Path) -> Option<([u8; 32], u64)> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut h = Sha256::new()?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        total += n as u64;
        h.update(&buf[..n]);
    }
    Some((h.finish(), total))
}

/// ¿`signature` (r ‖ s, 64 bytes) es una firma ECDSA P-256 con SHA-256 de
/// `message` por la clave pública `key` (X ‖ Y, 64 bytes)?
pub fn verify_p256(key: &[u8; 64], message: &[u8], signature: &[u8; 64]) -> bool {
    let Some(cng) = cng() else { return false };
    let Some(digest) = sha256(message) else { return false };
    let mut alg: Handle = std::ptr::null_mut();
    // SAFETY: punteros a variables locales.
    if unsafe { (cng.open)(&mut alg, w!("ECDSA_P256"), PCWSTR::null(), 0) } != 0 {
        return false;
    }
    // BCRYPT_ECCKEY_BLOB { magic = BCRYPT_ECDSA_PUBLIC_P256_MAGIC, cbKey = 32 } + X + Y.
    let mut blob = Vec::with_capacity(8 + 64);
    blob.extend_from_slice(&0x3153_4345u32.to_le_bytes());
    blob.extend_from_slice(&32u32.to_le_bytes());
    blob.extend_from_slice(key);
    let mut hkey: Handle = std::ptr::null_mut();
    // SAFETY: blob con el formato de bcrypt.h; proveedor abierto arriba.
    let imported = unsafe {
        (cng.import)(
            alg,
            std::ptr::null_mut(),
            w!("ECCPUBLICBLOB"),
            &mut hkey,
            blob.as_ptr(),
            blob.len() as u32,
            0,
        )
    } == 0;
    let ok = imported
        // SAFETY: clave importada; buffers locales con su largo.
        && unsafe {
            (cng.verify)(hkey, std::ptr::null(), digest.as_ptr(), 32, signature.as_ptr(), 64, 0)
        } == 0;
    // SAFETY: objetos propios.
    unsafe {
        if imported {
            (cng.destroy_key)(hkey);
        }
        (cng.close)(alg, 0);
    }
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Clave pública de `tests/update/test-key.pem` y una firma de openssl
    /// (`openssl dgst -sha256 -sign`) de "hola wm".
    const PUB: &str = "8fd2ee4ccdfb48453096d041b0409c3bca887d6ba846201a5786e5788d840f76b2918dda214f18e7077fd283a00b7545015eb0436d10a558c2357c835fdd4408";
    const SIG: &str = "3045022100abc24fb8c05496cbefbf9de75c4ca29154ee763322fc5b9b7437fa25413ed45b022058f4d75d53d9b44df5e972bc026f69fe409719f4ff01fd06695e7f5ee3692923";

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn openssl_signatures_verify() {
        let key: [u8; 64] = unhex(PUB).try_into().unwrap();
        let sig = wm_model::update::der_to_raw(&unhex(SIG)).unwrap();
        assert!(verify_p256(&key, b"hola wm", &sig));
        assert!(!verify_p256(&key, b"hola wM", &sig), "mensaje cambiado");
        let mut bad = sig;
        bad[10] ^= 1;
        assert!(!verify_p256(&key, b"hola wm", &bad), "firma cambiada");
        let mut other = key;
        other[63] ^= 1;
        assert!(!verify_p256(&other, b"hola wm", &sig), "otra clave");
    }

    #[test]
    fn sha256_vectors() {
        let h = |s: &str| wm_model::update::hex(&sha256(s.as_bytes()).unwrap());
        assert_eq!(h(""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(h("abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let mut inc = Sha256::new().unwrap();
        inc.update(b"a");
        inc.update(b"bc");
        assert_eq!(wm_model::update::hex(&inc.finish()), h("abc"));
    }
}
