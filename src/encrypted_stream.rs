use std::convert::TryInto;

use rand::Rng;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::crypto::{create_crypter, CipherType, Crypter, NonceType};
use crate::Error;
use crate::Result;

const LENGTH_SIZE: usize = 2;

pub async fn read_and_derive_crypter(
    stream: &mut (impl AsyncRead + std::marker::Unpin),
    master_key: &[u8],
    cipher_type: CipherType,
) -> Result<Box<dyn Crypter>> {
    let cipher_spec = cipher_type.spec();
    let mut salt = vec![0u8; cipher_spec.salt_size];
    stream.read_exact(&mut salt).await?;

    let subkey = crate::crypto::derive_subkey_compatible(master_key, &salt, cipher_spec.key_size);
    let crypter = create_crypter(&subkey, NonceType::Sequential, cipher_type);
    Ok(crypter)
}

pub async fn read_encrypt(
    stream: &mut (impl AsyncRead + std::marker::Unpin),
    crypter: &mut Box<dyn Crypter>,
) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; crypter.expected_ciphertext_length(LENGTH_SIZE)];
    stream.read_exact(buf.as_mut_slice()).await?;

    let length_bytes = crypter.decrypt(buf.as_slice())?;
    let length = u16::from_be_bytes(length_bytes.as_slice().try_into().unwrap()) as usize;

    let mut body_buf = vec![0u8; crypter.expected_ciphertext_length(length)];
    stream.read_exact(body_buf.as_mut_slice()).await?;

    crypter.decrypt(body_buf.as_slice())
}

pub async fn build_and_write_crypter(
    stream: &mut (impl AsyncWrite + std::marker::Unpin),
    master_key: &[u8],
    cipher_type: CipherType,
) -> Result<Box<dyn Crypter>> {
    let cipher_spec = cipher_type.spec();
    let mut salt = vec![0u8; cipher_spec.salt_size];
    (rand::os::OsRng::new()?).fill_bytes(&mut salt);
    stream.write_all(&salt).await?;

    let subkey = crate::crypto::derive_subkey_compatible(master_key, &salt, cipher_spec.key_size);
    let crypter = create_crypter(&subkey, NonceType::Sequential, cipher_type);
    Ok(crypter)
}

pub async fn write_encrypt(
    stream: &mut (impl AsyncWrite + std::marker::Unpin),
    crypter: &mut Box<dyn Crypter>,
    data: &[u8],
) -> Result<()> {
    let length = (data.len() as u16).to_be_bytes();
    let length_cipher_text = crypter.encrypt(&length)?;
    let ciphertext = crypter.encrypt(data)?;

    stream.write_all(length_cipher_text.as_slice()).await?;
    stream.write_all(ciphertext.as_slice()).await?;
    Ok(())
}

pub struct EncryptedStream {
    stream: TcpStream,
    decrypter: Box<dyn Crypter>,
    encrypter: Box<dyn Crypter>,
}

impl EncryptedStream {
    pub fn create(
        stream: TcpStream,
        decrypter: Box<dyn Crypter>,
        encrypter: Box<dyn Crypter>,
    ) -> Self {
        Self {
            stream,
            decrypter,
            encrypter,
        }
    }

    pub async fn read_package(&mut self) -> Result<Vec<u8>> {
        read_encrypt(&mut self.stream, &mut self.decrypter).await
    }

    pub async fn write_package(&mut self, data: &[u8]) -> Result<()> {
        write_encrypt(&mut self.stream, &mut self.encrypter, data).await
    }
}

fn convert_to_io_error(error: Error) -> std::io::Error {
    match error {
        Error::IOError(e) => e,
        Error::EncryptionError => {
            std::io::Error::new(std::io::ErrorKind::Other, "Encryption error")
        }
        Error::DecryptionError => {
            std::io::Error::new(std::io::ErrorKind::Other, "Decryption error")
        }
        e => {
            log::error!("Error decrypting stream {:?}", e);
            std::io::Error::new(std::io::ErrorKind::Other, "Unexpected error")
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::test_utils::ready_buf::ReadyBuf;

    #[tokio::test]
    async fn test_read_and_derive_crypter() -> Result<()> {
        let data = [0x1; 32];
        let mut crypter =
            read_and_derive_crypter(&mut data.as_ref(), b"key", CipherType::Aes256GCM).await?;

        let plaintext = [0x2; 37];
        let ciphertext = crypter.encrypt(&plaintext)?;

        assert_eq!(
            ciphertext,
            [
                0xD7, 0x73, 0x7E, 0x7C, 0xE0, 0xED, 0x78, 0xB2,
                0xB4, 0x03, 0x60, 0x52, 0x30, 0xF9, 0x96, 0xBE,
                0x97, 0xD9, 0x7B, 0xA4, 0xA1, 0xD4, 0x8F, 0xCD,
                0x85, 0xA8, 0xDF, 0xB2, 0xEB, 0xC4, 0xA6, 0xD6,
                0x5C, 0xE3, 0x39, 0xE6, 0xD8, 0x4A, 0x8D, 0xEE,
                0x75, 0xE2, 0x84, 0x66, 0x6A, 0x95, 0xC6, 0xEA,
                0xBC, 0x07, 0x99, 0x30, 0x20
            ]
            .to_vec(),
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_read_and_decrypt() -> Result<()> {
        let key = [0x1; 32];
        let mut crypter = create_crypter(&key, NonceType::Sequential, CipherType::Aes256GCM);
        let data = [
            0xBF, 0x10, 0xCC, 0xF3, 0xC4, 0x53, 0xE9, 0xBB,
            0x67, 0x94, 0xAB, 0x1F, 0x17, 0x79, 0xE2, 0x0B,
            0x64, 0xD2, 0x55, 0x95, 0x1E, 0x3A, 0x2A, 0x5C,
            0xFD, 0x58, 0xD5, 0x27, 0xAF, 0xB0, 0xA4, 0x11,
            0xFE, 0x61, 0x33, 0x8E, 0xFA, 0xC4, 0xCA, 0x73,
            0xD1, 0xEC, 0xF4, 0xBB, 0xE4, 0x92, 0xC2, 0x78,
            0x5B, 0x32, 0x4E, 0x26, 0x2A, 0x95, 0x74, 0xF3,
            0xAC, 0x74, 0x9C, 0x5E, 0x77, 0xF2, 0x5F, 0x7A,
            0x18, 0x0D, 0x52, 0x69, 0xE3, 0xE0, 0x40,
        ];

        let decrypted_text = read_encrypt(&mut data.as_ref(), &mut crypter).await?;

        let plaintext = [0x2; 37];
        assert_eq!(decrypted_text, plaintext.to_vec());

        Ok(())
    }

    #[tokio::test]
    async fn test_build_and_write_crypter() -> Result<()> {
        let mut ready_buf = ReadyBuf::make(&[]);
        let ciphertext = {
            let mut encrypter =
                build_and_write_crypter(&mut ready_buf, b"key", CipherType::Aes256GCM).await?;
            encrypter.encrypt(&[1u8; 32])?
        };

        let data = ready_buf.combined();
        assert_ne!(data, [0u8; 32]);

        let plaintext = {
            let mut read_buf = data.as_ref();
            let mut decrypter =
                read_and_derive_crypter(&mut read_buf, b"key", CipherType::Aes256GCM).await?;
            decrypter.decrypt(ciphertext.as_slice())?
        };

        assert_eq!(plaintext, &[1u8; 32]);

        Ok(())
    }

    #[tokio::test]
    async fn test_write_encrypt() -> Result<()> {
        let key = [0x1; 32];
        let mut crypter = create_crypter(&key, NonceType::Sequential, CipherType::Aes256GCM);

        let mut ready_buf = ReadyBuf::make(&[]);
        write_encrypt(&mut ready_buf, &mut crypter, &[0x02; 37]).await?;
        assert_eq!(
            ready_buf.combined(),
            [
                0xBF, 0x10, 0xCC, 0xF3, 0xC4, 0x53, 0xE9, 0xBB,
                0x67, 0x94, 0xAB, 0x1F, 0x17, 0x79, 0xE2, 0x0B,
                0x64, 0xD2, 0x55, 0x95, 0x1E, 0x3A, 0x2A, 0x5C,
                0xFD, 0x58, 0xD5, 0x27, 0xAF, 0xB0, 0xA4, 0x11,
                0xFE, 0x61, 0x33, 0x8E, 0xFA, 0xC4, 0xCA, 0x73,
                0xD1, 0xEC, 0xF4, 0xBB, 0xE4, 0x92, 0xC2, 0x78,
                0x5B, 0x32, 0x4E, 0x26, 0x2A, 0x95, 0x74, 0xF3,
                0xAC, 0x74, 0x9C, 0x5E, 0x77, 0xF2, 0x5F, 0x7A,
                0x18, 0x0D, 0x52, 0x69, 0xE3, 0xE0, 0x40
            ]
            .to_vec(),
        );

        Ok(())
    }
}