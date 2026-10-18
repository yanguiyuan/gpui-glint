/** Glint plugin API v1. No Node, filesystem, process or network APIs are implicit. */
declare const glint: {
  readonly apiVersion: 1;
  on(name: string, callback: (payload: any) => unknown): void;
  readonly host: { call(name: string, ...args: unknown[]): any };
};

/** Crypto host calls require the `crypto` capability. */
// crypto.md5Hex(text): lowercase hexadecimal digest.
// crypto.hmacSha256VerifyBase64Url(key, text, signature): boolean; no Base64 padding.
// crypto.aes256CbcPkcs7DecryptBase64(key, iv, ciphertext): UTF-8 plaintext, strict padding.
