use alloc::string::{String, ToString};
use core::fmt::Display;
use crc::{Crc, CRC_32_CKSUM};
use spacepackets::cfdp::ChecksumType;
use spacepackets::ByteConversionError;
#[cfg(feature = "std")]
use std::error::Error;
#[cfg(feature = "std")]
pub use stdmod::*;

pub const CRC_32: Crc<u32> = Crc::<u32>::new(&CRC_32_CKSUM);

#[derive(Debug, Clone)]
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

impl Display for FilestoreError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            FilestoreError::FileDoesNotExist => {
                write!(f, "file does not exist")
            }
            FilestoreError::FileAlreadyExists => {
                write!(f, "file already exists")
            }
            FilestoreError::DirDoesNotExist => {
                write!(f, "directory does not exist")
            }
            FilestoreError::Permission => {
                write!(f, "permission error")
            }
            FilestoreError::IsNotFile => {
                write!(f, "is not a file")
            }
            FilestoreError::IsNotDirectory => {
                write!(f, "is not a directory")
            }
            FilestoreError::ByteConversion(e) => {
                write!(f, "filestore error: {e}")
            }
            FilestoreError::Io { raw_errno, string } => {
                write!(
                    f,
                    "filestore generic IO error with raw errno {:?}: {}",
                    raw_errno, string
                )
            }
            FilestoreError::ChecksumTypeNotImplemented(checksum_type) => {
                write!(f, "checksum {:?} not implemented", checksum_type)
            }
        }
    }
}

impl Error for FilestoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            FilestoreError::ByteConversion(e) => Some(e),
            _ => None,
        }
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
            let mut file = OpenOptions::new().write(true).open(file)?;
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
    use std::{fs, path::Path, println};

    use super::*;
    use tempfile::tempdir;

    const NATIVE_FS: NativeFilestore = NativeFilestore {};

    #[test]
    fn test_basic_native_filestore_create() {
        let tmpdir = tempdir().expect("creating tmpdir failed");
        let file_path = tmpdir.path().join("test.txt");
        let result =
            NATIVE_FS.create_file(file_path.to_str().expect("getting str for file failed"));
        assert!(result.is_ok());
        let path = Path::new(&file_path);
        assert!(path.exists());
        assert!(NATIVE_FS.exists(file_path.to_str().unwrap()));
        assert!(NATIVE_FS.is_file(file_path.to_str().unwrap()));
        fs::remove_dir_all(tmpdir).expect("clearing tmpdir failed");
    }

    #[test]
    fn test_basic_native_fs_exists() {
        let tmpdir = tempdir().expect("creating tmpdir failed");
        let file_path = tmpdir.path().join("test.txt");
        assert!(!NATIVE_FS.exists(file_path.to_str().unwrap()));
        NATIVE_FS
            .create_file(file_path.to_str().expect("getting str for file failed"))
            .unwrap();
        assert!(NATIVE_FS.exists(file_path.to_str().unwrap()));
        assert!(NATIVE_FS.is_file(file_path.to_str().unwrap()));
        fs::remove_dir_all(tmpdir).expect("clearing tmpdir failed");
    }

    #[test]
    fn test_basic_native_fs_write() {
        let tmpdir = tempdir().expect("creating tmpdir failed");
        let file_path = tmpdir.path().join("test.txt");
        assert!(!NATIVE_FS.exists(file_path.to_str().unwrap()));
        NATIVE_FS
            .create_file(file_path.to_str().expect("getting str for file failed"))
            .unwrap();
        assert!(NATIVE_FS.exists(file_path.to_str().unwrap()));
        assert!(NATIVE_FS.is_file(file_path.to_str().unwrap()));
        println!("{}", file_path.to_str().unwrap());
        let write_data = "hello world\n";
        NATIVE_FS
            .write_data(file_path.to_str().unwrap(), 0, write_data.as_bytes())
            .expect("writing to file failed");
        let read_back = fs::read_to_string(file_path).expect("reading back data failed");
        assert_eq!(read_back, write_data);
        fs::remove_dir_all(tmpdir).expect("clearing tmpdir failed");
    }

    #[test]
    fn test_basic_native_fs_read() {
        let tmpdir = tempdir().expect("creating tmpdir failed");
        let file_path = tmpdir.path().join("test.txt");
        assert!(!NATIVE_FS.exists(file_path.to_str().unwrap()));
        NATIVE_FS
            .create_file(file_path.to_str().expect("getting str for file failed"))
            .unwrap();
        assert!(NATIVE_FS.exists(file_path.to_str().unwrap()));
        assert!(NATIVE_FS.is_file(file_path.to_str().unwrap()));
        println!("{}", file_path.to_str().unwrap());
        let write_data = "hello world\n";
        NATIVE_FS
            .write_data(file_path.to_str().unwrap(), 0, write_data.as_bytes())
            .expect("writing to file failed");
        let read_back = fs::read_to_string(file_path).expect("reading back data failed");
        assert_eq!(read_back, write_data);
        fs::remove_dir_all(tmpdir).expect("clearing tmpdir failed");
    }
}
