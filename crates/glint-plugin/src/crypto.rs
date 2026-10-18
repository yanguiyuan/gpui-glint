//! Generic, capability-gated crypto primitives. Provider protocols live in plugins.
use crate::{HostRegistry, PluginError};
use aes::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::{Digest, Sha256};

fn arg(args: &Value, index: usize) -> crate::Result<&str> {
    args.get(index)
        .and_then(Value::as_str)
        .ok_or_else(|| PluginError("Expected string".into()))
}
pub(crate) fn extend(host: &mut HostRegistry) {
    host.register("crypto.md5Hex", "crypto", |args| {
        Ok(Value::String(format!(
            "{:x}",
            md5::Md5::digest(arg(&args, 0)?.as_bytes())
        )))
    })
    .expect("unique builtin");
    host.register("crypto.hmacSha256VerifyBase64Url", "crypto", |args| {
        let mut mac = Hmac::<Sha256>::new_from_slice(arg(&args, 0)?.as_bytes())
            .map_err(|_| PluginError("Invalid key".into()))?;
        mac.update(arg(&args, 1)?.as_bytes());
        let valid = URL_SAFE_NO_PAD
            .decode(arg(&args, 2)?)
            .ok()
            .is_some_and(|signature| mac.verify_slice(&signature).is_ok());
        Ok(Value::Bool(valid))
    })
    .expect("unique builtin");
    host.register("crypto.aes256CbcPkcs7DecryptBase64", "crypto", |args| {
        let key = arg(&args, 0)?.as_bytes();
        let iv = arg(&args, 1)?
            .as_bytes()
            .get(..16)
            .ok_or_else(|| PluginError("Invalid IV".into()))?;
        let mut ciphertext = STANDARD
            .decode(arg(&args, 2)?)
            .map_err(|_| PluginError("Invalid ciphertext".into()))?;
        let cipher = cbc::Decryptor::<aes::Aes256>::new_from_slices(key, iv)
            .map_err(|_| PluginError("Invalid key or IV".into()))?;
        let plain = cipher
            .decrypt_padded_mut::<Pkcs7>(&mut ciphertext)
            .map_err(|_| PluginError("Invalid ciphertext or padding".into()))?;
        let text = std::str::from_utf8(plain).map_err(|_| PluginError("Invalid UTF-8".into()))?;
        Ok(Value::String(text.into()))
    })
    .expect("unique builtin");
}
