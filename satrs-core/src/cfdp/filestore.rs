use alloc::string::{String, ToString};
use crc::{Crc, CRC_32_CKSUM};
use spacepackets::cfdp::ChecksumType;
use spacepackets::ByteConversionError;
#[cfg(feature = "std")]
pub use stdmod::*;

pub const CRC_32: Crc<u32> = Crc::<u32>::new(&CRC_32_CKSUM);

pub enum FilestoreError {
    FileDoesNotExist,
    FileAlreadyExists,
    DirDoesNotExist,
    Permission,
    IsNotFile,
    IsNotDirectory,
    ByteConversion(ByteConversionError),
    Io {
        raw_errno: Option<i32>,
        string: String,
    },
    ChecksumTypeNotImplemented(ChecksumType),
}

impl From<ByteConversionError> for FilestoreError {
    fn from(value: ByteConversionError) -> Self {
        Self::ByteConversion(value)
    }
}

#[cfg(feature = "std")]
impl From<std::io::Error> for FilestoreError {
    fn from(value: std::io::Error) -> Self {
        Self::Io {
            raw_errno: value.raw_os_error(),
            string: value.to_string(),
        }
    }
}

pub trait VirtualFilestore {
    fn create_file(&self, file_path: &str) -> Result<(), FilestoreError>;

    fn remove_file(&self, file_path: &str) -> Result<(), FilestoreError>;

    /// Truncating a file means deleting all its data so the resulting file is empty.
    /// This can be more efficient than removing and re-creating a file.
    fn truncate_file(&self, file_path: &str) -> Result<(), FilestoreError>;

    fn remove_dir(&self, file_path: &str, all: bool) -> Result<(), FilestoreError>;

    fn read_data(
        &self,
        file_path: &str,
        offset: u64,
        read_len: u64,
        buf: &mut [u8],
    ) -> Result<(), FilestoreError>;

    fn write_data(&self, file: &str, offset: u64, buf: &[u8]) -> Result<(), FilestoreError>;

    fn filename_from_full_path<'a>(&self, path: &'a str) -> Option<&'a str>;

    fn is_file(&self, path: &str) -> bool;

    fn is_dir(&self, path: &str) -> bool {
        !self.is_file(path)
    }

    fn exists(&self, path: &str) -> bool;

    /// This special function is the CFDP specific abstraction to verify the checksum of a file.
    /// This allows to keep OS specific details like reading the whole file in the most efficient
    /// manner inside the file system abstraction.
    fn checksum_verify(
        &self,
        file_path: &str,
        checksum_type: ChecksumType,
        expected_checksum: u32,
        verification_buf: &mut [u8],
    ) -> Result<bool, FilestoreError>;
}

#[cfg(feature = "std")]
pub mod stdmod {
    use super::*;
    use std::{
        fs::{self, File, OpenOptions},
        io::{BufReader, Read, Seek, SeekFrom, Write},
        path::Path,
    };

    pub struct NativeFilestore {}

    impl VirtualFilestore for NativeFilestore {
        fn create_file(&self, file_path: &str) -> Result<(), FilestoreError> {
            if self.exists(file_path) {
                return Err(FilestoreError::FileAlreadyExists);
            }
            File::create(file_path)?;
            Ok(())
        }

        fn remove_file(&self, file_path: &str) -> Result<(), FilestoreError> {
            if !self.exists(file_path) {
                return Ok(());
            }
            if !self.is_file(file_path) {
                return Err(FilestoreError::IsNotFile);
            }
            fs::remove_file(file_path)?;
            Ok(())
        }

        fn truncate_file(&self, file_path: &str) -> Result<(), FilestoreError> {
            if !self.exists(file_path) {
                return Ok(());
            }
            if !self.is_file(file_path) {
                return Err(FilestoreError::IsNotFile);
            }
            OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(file_path)?;
            Ok(())
        }

        fn remove_dir(&self, dir_path: &str, all: bool) -> Result<(), FilestoreError> {
            if !self.exists(dir_path) {
                return Err(FilestoreError::DirDoesNotExist);
            }
            if !self.is_dir(dir_path) {
                return Err(FilestoreError::IsNotDirectory);
            }
            if !all {
                fs::remove_dir(dir_path)?;
                return Ok(());
            }
            fs::remove_dir_all(dir_path)?;
            Ok(())
        }

        fn read_data(
            &self,
            file_name: &str,
            offset: u64,
            read_len: u64,
            buf: &mut [u8],
        ) -> Result<(), FilestoreError> {
            if buf.len() < read_len as usize {
                return Err(ByteConversionError::ToSliceTooSmall {
                    found: buf.len(),
                    expected: read_len as usize,
                }
                .into());
            }
            if !self.exists(file_name) {
                return Err(FilestoreError::FileDoesNotExist);
            }
            if !self.is_file(file_name) {
                return Err(FilestoreError::IsNotFile);
            }
            let mut file = File::open(file_name)?;
            file.seek(SeekFrom::Start(offset))?;
            file.read_exact(&mut buf[0..read_len as usize])?;
            Ok(())
        }

        fn write_data(&self, file: &str, offset: u64, buf: &[u8]) -> Result<(), FilestoreError> {
            if !self.exists(file) {
                return Err(FilestoreError::FileDoesNotExist);
            }
            if !self.is_file(file) {
                return Err(FilestoreError::IsNotFile);
            }
            let mut file = File::open(file)?;
            file.seek(SeekFrom::Start(offset))?;
            file.write_all(buf)?;
            Ok(())
        }

        fn filename_from_full_path<'a>(&self, path: &'a str) -> Option<&'a str> {
            // Convert the path string to a Path
            let path = Path::new(path);

            // Extract the file name using the file_name() method
            path.file_name().and_then(|name| name.to_str())
        }

        fn is_file(&self, path: &str) -> bool {
            let path = Path::new(path);
            path.is_file()
        }

        fn is_dir(&self, path: &str) -> bool {
            let path = Path::new(path);
            path.is_dir()
        }

        fn exists(&self, path: &str) -> bool {
            let path = Path::new(path);
            if !path.exists() {
                return false;
            }
            true
        }

        fn checksum_verify(
            &self,
            file_path: &str,
            checksum_type: ChecksumType,
            expected_checksum: u32,
            verification_buf: &mut [u8],
        ) -> Result<bool, FilestoreError> {
            match checksum_type {
                ChecksumType::Modular => {
                    todo!();
                }
                ChecksumType::Crc32 => {
                    let mut digest = CRC_32.digest();
                    let file_to_check = File::open(file_path)?;
                    let mut buf_reader = BufReader::new(file_to_check);
                    loop {
                        let bytes_read = buf_reader.read(verification_buf)?;
                        if bytes_read == 0 {
                            break;
                        }
                        digest.update(&verification_buf[0..bytes_read]);
                    }
                    if digest.finalize() == expected_checksum {
                        return Ok(true);
                    }
                    Ok(false)
                }
                ChecksumType::NullChecksum => Ok(true),
                _ => Err(FilestoreError::ChecksumTypeNotImplemented(checksum_type)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_basic_native_filestore() {
        let tmpdir = tempdir().expect("creating tmpdir failed");
        let file_path = tmpdir.path().join("test.txt");
        let native_filestore = NativeFilestore {};
        let result =
            native_filestore.create_file(file_path.to_str().expect("getting str for file failed"));
        assert!(result.is_ok());
    }
}
