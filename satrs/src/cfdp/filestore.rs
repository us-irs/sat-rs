use alloc::string::{String, ToString};
use core::fmt::Display;
use crc::{Crc, CRC_32_CKSUM};
use spacepackets::cfdp::ChecksumType;
use spacepackets::ByteConversionError;
#[cfg(feature = "std")]
use std::error::Error;
use std::path::Path;
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

    fn remove_dir(&self, dir_path: &str, all: bool) -> Result<(), FilestoreError>;
    fn create_dir(&self, dir_path: &str) -> Result<(), FilestoreError>;

    fn read_data(
        &self,
        file_path: &str,
        offset: u64,
        read_len: u64,
        buf: &mut [u8],
    ) -> Result<(), FilestoreError>;

    fn write_data(&self, file: &str, offset: u64, buf: &[u8]) -> Result<(), FilestoreError>;

    fn filename_from_full_path(path: &str) -> Option<&str>
    where
        Self: Sized,
    {
        // Convert the path string to a Path
        let path = Path::new(path);

        // Extract the file name using the file_name() method
        path.file_name().and_then(|name| name.to_str())
    }

    fn is_file(&self, path: &str) -> bool;

    fn is_dir(&self, path: &str) -> bool {
        !self.is_file(path)
    }

    fn exists(&self, path: &str) -> bool;

    /// This special function is the CFDP specific abstraction to verify the checksum of a file.
    /// This allows to keep OS specific details like reading the whole file in the most efficient
    /// manner inside the file system abstraction.
    ///
    /// The passed verification buffer argument will be used by the specific implementation as
    /// a buffer to read the file into. It is recommended to use common buffer sizes like
    /// 4096 or 8192 bytes.
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

    #[derive(Default)]
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
                return Err(FilestoreError::FileDoesNotExist);
            }
            if !self.is_file(file_path) {
                return Err(FilestoreError::IsNotFile);
            }
            fs::remove_file(file_path)?;
            Ok(())
        }

        fn truncate_file(&self, file_path: &str) -> Result<(), FilestoreError> {
            if !self.exists(file_path) {
                return Err(FilestoreError::FileDoesNotExist);
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

        fn create_dir(&self, dir_path: &str) -> Result<(), FilestoreError> {
            fs::create_dir(dir_path).map_err(|e| FilestoreError::Io {
                raw_errno: e.raw_os_error(),
                string: e.to_string(),
            })?;
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

        fn is_file(&self, path: &str) -> bool {
            let path = Path::new(path);
            path.is_file()
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
                    if self.calc_modular_checksum(file_path)? == expected_checksum {
                        return Ok(true);
                    }
                    Ok(false)
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

    impl NativeFilestore {
        pub fn calc_modular_checksum(&self, file_path: &str) -> Result<u32, FilestoreError> {
            let mut checksum: u32 = 0;
            let file = File::open(file_path)?;
            let mut buf_reader = BufReader::new(file);
            let mut buffer = [0; 4];

            loop {
                let bytes_read = buf_reader.read(&mut buffer)?;
                if bytes_read == 0 {
                    break;
                }
                // Perform padding directly in the buffer
                (bytes_read..4).for_each(|i| {
                    buffer[i] = 0;
                });

                checksum = checksum.wrapping_add(u32::from_be_bytes(buffer));
            }
            Ok(checksum)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path, println};

    use super::*;
    use alloc::format;
    use tempfile::tempdir;

    const EXAMPLE_DATA_CFDP: [u8; 15] = [
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E,
    ];

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
    }

    #[test]
    fn test_basic_native_fs_file_exists() {
        let tmpdir = tempdir().expect("creating tmpdir failed");
        let file_path = tmpdir.path().join("test.txt");
        assert!(!NATIVE_FS.exists(file_path.to_str().unwrap()));
        NATIVE_FS
            .create_file(file_path.to_str().expect("getting str for file failed"))
            .unwrap();
        assert!(NATIVE_FS.exists(file_path.to_str().unwrap()));
        assert!(NATIVE_FS.is_file(file_path.to_str().unwrap()));
    }

    #[test]
    fn test_basic_native_fs_dir_exists() {
        let tmpdir = tempdir().expect("creating tmpdir failed");
        let dir_path = tmpdir.path().join("testdir");
        assert!(!NATIVE_FS.exists(dir_path.to_str().unwrap()));
        NATIVE_FS
            .create_dir(dir_path.to_str().expect("getting str for file failed"))
            .unwrap();
        assert!(NATIVE_FS.exists(dir_path.to_str().unwrap()));
        assert!(NATIVE_FS.is_dir(dir_path.as_path().to_str().unwrap()));
    }

    #[test]
    fn test_basic_native_fs_remove_file() {
        let tmpdir = tempdir().expect("creating tmpdir failed");
        let file_path = tmpdir.path().join("test.txt");
        NATIVE_FS
            .create_file(file_path.to_str().expect("getting str for file failed"))
            .expect("creating file failed");
        assert!(NATIVE_FS.exists(file_path.to_str().unwrap()));
        NATIVE_FS
            .remove_file(file_path.to_str().unwrap())
            .expect("removing file failed");
        assert!(!NATIVE_FS.exists(file_path.to_str().unwrap()));
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
    }

    #[test]
    fn test_truncate_file() {
        let tmpdir = tempdir().expect("creating tmpdir failed");
        let file_path = tmpdir.path().join("test.txt");
        NATIVE_FS
            .create_file(file_path.to_str().expect("getting str for file failed"))
            .expect("creating file failed");
        fs::write(file_path.clone(), [1, 2, 3, 4]).unwrap();
        assert_eq!(fs::read(file_path.clone()).unwrap(), [1, 2, 3, 4]);
        NATIVE_FS
            .truncate_file(file_path.to_str().unwrap())
            .unwrap();
        assert_eq!(fs::read(file_path.clone()).unwrap(), []);
    }

    #[test]
    fn test_remove_dir() {
        let tmpdir = tempdir().expect("creating tmpdir failed");
        let dir_path = tmpdir.path().join("testdir");
        assert!(!NATIVE_FS.exists(dir_path.to_str().unwrap()));
        NATIVE_FS
            .create_dir(dir_path.to_str().expect("getting str for file failed"))
            .unwrap();
        assert!(NATIVE_FS.exists(dir_path.to_str().unwrap()));
        NATIVE_FS
            .remove_dir(dir_path.to_str().unwrap(), false)
            .unwrap();
        assert!(!NATIVE_FS.exists(dir_path.to_str().unwrap()));
    }

    #[test]
    fn test_read_file() {
        let tmpdir = tempdir().expect("creating tmpdir failed");
        let file_path = tmpdir.path().join("test.txt");
        NATIVE_FS
            .create_file(file_path.to_str().expect("getting str for file failed"))
            .expect("creating file failed");
        fs::write(file_path.clone(), [1, 2, 3, 4]).unwrap();
        let read_buf: &mut [u8] = &mut [0; 4];
        NATIVE_FS
            .read_data(file_path.to_str().unwrap(), 0, 4, read_buf)
            .unwrap();
        assert_eq!([1, 2, 3, 4], read_buf);
        NATIVE_FS
            .write_data(file_path.to_str().unwrap(), 4, &[5, 6, 7, 8])
            .expect("writing to file failed");
        NATIVE_FS
            .read_data(file_path.to_str().unwrap(), 2, 4, read_buf)
            .unwrap();
        assert_eq!([3, 4, 5, 6], read_buf);
    }

    #[test]
    fn test_remove_which_does_not_exist() {
        let tmpdir = tempdir().expect("creating tmpdir failed");
        let file_path = tmpdir.path().join("test.txt");
        let result = NATIVE_FS.read_data(file_path.to_str().unwrap(), 0, 4, &mut [0; 4]);
        assert!(result.is_err());
        let error = result.unwrap_err();
        if let FilestoreError::FileDoesNotExist = error {
            assert_eq!(error.to_string(), "file does not exist");
        } else {
            panic!("unexpected error");
        }
    }

    #[test]
    fn test_file_already_exists() {
        let tmpdir = tempdir().expect("creating tmpdir failed");
        let file_path = tmpdir.path().join("test.txt");
        let result =
            NATIVE_FS.create_file(file_path.to_str().expect("getting str for file failed"));
        assert!(result.is_ok());
        let result =
            NATIVE_FS.create_file(file_path.to_str().expect("getting str for file failed"));
        assert!(result.is_err());
        let error = result.unwrap_err();
        if let FilestoreError::FileAlreadyExists = error {
            assert_eq!(error.to_string(), "file already exists");
        } else {
            panic!("unexpected error");
        }
    }

    #[test]
    fn test_remove_file_with_dir_api() {
        let tmpdir = tempdir().expect("creating tmpdir failed");
        let file_path = tmpdir.path().join("test.txt");
        NATIVE_FS
            .create_file(file_path.to_str().expect("getting str for file failed"))
            .unwrap();
        let result = NATIVE_FS.remove_dir(file_path.to_str().unwrap(), true);
        assert!(result.is_err());
        let error = result.unwrap_err();
        if let FilestoreError::IsNotDirectory = error {
            assert_eq!(error.to_string(), "is not a directory");
        } else {
            panic!("unexpected error");
        }
    }

    #[test]
    fn test_remove_dir_remove_all() {
        let tmpdir = tempdir().expect("creating tmpdir failed");
        let dir_path = tmpdir.path().join("test");
        NATIVE_FS
            .create_dir(dir_path.to_str().expect("getting str for file failed"))
            .unwrap();
        let file_path = dir_path.as_path().join("test.txt");
        NATIVE_FS
            .create_file(file_path.to_str().expect("getting str for file failed"))
            .unwrap();
        let result = NATIVE_FS.remove_dir(dir_path.to_str().unwrap(), true);
        assert!(result.is_ok());
        assert!(!NATIVE_FS.exists(dir_path.to_str().unwrap()));
    }

    #[test]
    fn test_remove_dir_with_file_api() {
        let tmpdir = tempdir().expect("creating tmpdir failed");
        let file_path = tmpdir.path().join("test");
        NATIVE_FS
            .create_dir(file_path.to_str().expect("getting str for file failed"))
            .unwrap();
        let result = NATIVE_FS.remove_file(file_path.to_str().unwrap());
        assert!(result.is_err());
        let error = result.unwrap_err();
        if let FilestoreError::IsNotFile = error {
            assert_eq!(error.to_string(), "is not a file");
        } else {
            panic!("unexpected error");
        }
    }

    #[test]
    fn test_remove_dir_which_does_not_exist() {
        let tmpdir = tempdir().expect("creating tmpdir failed");
        let file_path = tmpdir.path().join("test");
        let result = NATIVE_FS.remove_dir(file_path.to_str().unwrap(), true);
        assert!(result.is_err());
        let error = result.unwrap_err();
        if let FilestoreError::DirDoesNotExist = error {
            assert_eq!(error.to_string(), "directory does not exist");
        } else {
            panic!("unexpected error");
        }
    }

    #[test]
    fn test_remove_file_which_does_not_exist() {
        let tmpdir = tempdir().expect("creating tmpdir failed");
        let file_path = tmpdir.path().join("test.txt");
        let result = NATIVE_FS.remove_file(file_path.to_str().unwrap());
        assert!(result.is_err());
        let error = result.unwrap_err();
        if let FilestoreError::FileDoesNotExist = error {
            assert_eq!(error.to_string(), "file does not exist");
        } else {
            panic!("unexpected error");
        }
    }

    #[test]
    fn test_truncate_file_which_does_not_exist() {
        let tmpdir = tempdir().expect("creating tmpdir failed");
        let file_path = tmpdir.path().join("test.txt");
        let result = NATIVE_FS.truncate_file(file_path.to_str().unwrap());
        assert!(result.is_err());
        let error = result.unwrap_err();
        if let FilestoreError::FileDoesNotExist = error {
            assert_eq!(error.to_string(), "file does not exist");
        } else {
            panic!("unexpected error");
        }
    }

    #[test]
    fn test_truncate_file_on_directory() {
        let tmpdir = tempdir().expect("creating tmpdir failed");
        let file_path = tmpdir.path().join("test");
        NATIVE_FS.create_dir(file_path.to_str().unwrap()).unwrap();
        let result = NATIVE_FS.truncate_file(file_path.to_str().unwrap());
        assert!(result.is_err());
        let error = result.unwrap_err();
        if let FilestoreError::IsNotFile = error {
            assert_eq!(error.to_string(), "is not a file");
        } else {
            panic!("unexpected error");
        }
    }

    #[test]
    fn test_byte_conversion_error_when_reading() {
        let tmpdir = tempdir().expect("creating tmpdir failed");
        let file_path = tmpdir.path().join("test.txt");
        NATIVE_FS
            .create_file(file_path.to_str().expect("getting str for file failed"))
            .unwrap();
        let result = NATIVE_FS.read_data(file_path.to_str().unwrap(), 0, 2, &mut []);
        assert!(result.is_err());
        let error = result.unwrap_err();
        if let FilestoreError::ByteConversion(byte_conv_error) = error {
            if let ByteConversionError::ToSliceTooSmall { found, expected } = byte_conv_error {
                assert_eq!(found, 0);
                assert_eq!(expected, 2);
            } else {
                panic!("unexpected error");
            }
            assert_eq!(
                error.to_string(),
                format!("filestore error: {}", byte_conv_error)
            );
        } else {
            panic!("unexpected error");
        }
    }

    #[test]
    fn test_read_file_on_dir() {
        let tmpdir = tempdir().expect("creating tmpdir failed");
        let dir_path = tmpdir.path().join("test");
        NATIVE_FS
            .create_dir(dir_path.to_str().expect("getting str for file failed"))
            .unwrap();
        let result = NATIVE_FS.read_data(dir_path.to_str().unwrap(), 0, 4, &mut [0; 4]);
        assert!(result.is_err());
        let error = result.unwrap_err();
        if let FilestoreError::IsNotFile = error {
            assert_eq!(error.to_string(), "is not a file");
        } else {
            panic!("unexpected error");
        }
    }

    #[test]
    fn test_write_file_non_existing() {
        let tmpdir = tempdir().expect("creating tmpdir failed");
        let file_path = tmpdir.path().join("test.txt");
        let result = NATIVE_FS.write_data(file_path.to_str().unwrap(), 0, &[]);
        assert!(result.is_err());
        let error = result.unwrap_err();
        if let FilestoreError::FileDoesNotExist = error {
        } else {
            panic!("unexpected error");
        }
    }

    #[test]
    fn test_write_file_on_dir() {
        let tmpdir = tempdir().expect("creating tmpdir failed");
        let file_path = tmpdir.path().join("test");
        NATIVE_FS.create_dir(file_path.to_str().unwrap()).unwrap();
        let result = NATIVE_FS.write_data(file_path.to_str().unwrap(), 0, &[]);
        assert!(result.is_err());
        let error = result.unwrap_err();
        if let FilestoreError::IsNotFile = error {
        } else {
            panic!("unexpected error");
        }
    }

    #[test]
    fn test_filename_extraction() {
        let tmpdir = tempdir().expect("creating tmpdir failed");
        let file_path = tmpdir.path().join("test.txt");
        NATIVE_FS
            .create_file(file_path.to_str().expect("getting str for file failed"))
            .unwrap();
        NativeFilestore::filename_from_full_path(file_path.to_str().unwrap());
    }

    #[test]
    fn test_modular_checksum() {
        let tmpdir = tempdir().expect("creating tmpdir failed");
        let file_path = tmpdir.path().join("mod-crc.bin");
        fs::write(file_path.as_path(), EXAMPLE_DATA_CFDP).expect("writing test file failed");
        // Kind of re-writing the modular checksum impl here which we are trying to test, but the
        // numbers/correctness were verified manually using calculators, so this is okay.
        let mut checksum: u32 = 0;
        let mut buffer: [u8; 4] = [0; 4];
        for i in 0..3 {
            buffer = EXAMPLE_DATA_CFDP[i * 4..(i + 1) * 4].try_into().unwrap();
            checksum = checksum.wrapping_add(u32::from_be_bytes(buffer));
        }
        buffer[0..3].copy_from_slice(&EXAMPLE_DATA_CFDP[12..15]);
        buffer[3] = 0;
        checksum = checksum.wrapping_add(u32::from_be_bytes(buffer));
        let mut verif_buf: [u8; 32] = [0; 32];
        let result = NATIVE_FS.checksum_verify(
            file_path.to_str().unwrap(),
            ChecksumType::Modular,
            checksum,
            &mut verif_buf,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_null_checksum_impl() {
        let tmpdir = tempdir().expect("creating tmpdir failed");
        let file_path = tmpdir.path().join("mod-crc.bin");
        // The file to check does not even need to exist, and the verification buffer can be
        // empty: the null checksum is always yields the same result.
        let result = NATIVE_FS.checksum_verify(
            file_path.to_str().unwrap(),
            ChecksumType::NullChecksum,
            0,
            &mut [],
        );
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn test_checksum_not_implemented() {
        let tmpdir = tempdir().expect("creating tmpdir failed");
        let file_path = tmpdir.path().join("mod-crc.bin");
        // The file to check does not even need to exist, and the verification buffer can be
        // empty: the null checksum is always yields the same result.
        let result = NATIVE_FS.checksum_verify(
            file_path.to_str().unwrap(),
            ChecksumType::Crc32Proximity1,
            0,
            &mut [],
        );
        assert!(result.is_err());
        let error = result.unwrap_err();
        if let FilestoreError::ChecksumTypeNotImplemented(cksum_type) = error {
            assert_eq!(
                error.to_string(),
                format!("checksum {:?} not implemented", cksum_type)
            );
        } else {
            panic!("unexpected error");
        }
    }
}
