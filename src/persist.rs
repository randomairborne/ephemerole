//! Ephemerole persistence system
//! .epd (**ep**hemerole **d**ata) file format
//!
//! All values are little endian.
//! 8 bytes of [`MAGIC`]
//! C: 8 bytes counting the number of entries in the database
//! 24 * C bytes of (userid:u64,messagecount:u64,lastmessageat:u64)
//! 8 bytes of checksum
//!
//! lastmessageat is in discord epoch seconds
use std::{
    io::{Error as IoError, ErrorKind as IoErrorKind, ErrorKind},
    ops::BitXor,
};

use ephemerole::{MessageMap, UserData};
use twilight_model::id::Id;

const MAGIC_BYTES: [u8; 8] = [0x85, 0x1E, 0x44, 0xB9, 0xA6, 0x58, 0x8F, 0x7F]; // Random bytes chosen to identify our custom filetype

/// Saves a [`MessageMap`] to the I/O object provided.
///
/// This function takes an I/O object to prevent serializing a ton of data to
/// memory before it is flushed to disk.
pub fn save(map: &MessageMap, file: &mut impl std::io::Write) -> Result<(), IoError> {
    let mut hash = Fnv1A::new();
    // Whenever we write something to the file, we have to update it in the hasher too.
    // This ensures data integrity.
    file.write_all(&MAGIC_BYTES)?;
    hash.update_each(&MAGIC_BYTES);

    // Convert the number of user data entries we have to a constant-width number
    // We throw up an error if we can't convert it. Not sure how that would happen, but..
    // 128-bit CPUs might exist someday.
    let entry_count: u64 = map
        .len()
        .try_into()
        .map_err(|_| IoError::new(ErrorKind::Other, "Entry count exceeds supported size."))?;
    let entry_count_bytes = entry_count.to_le_bytes(); // Convert the number of entries to raw bytes so we can store it, and won't need a special terminator.
    file.write_all(&entry_count_bytes)?;
    hash.update_each(&entry_count_bytes);

    for (id, data) in map {
        // We load things into this special data structure to keep all the encoding and decoding logic
        // in the same place inside that struct's implementation block.
        let save_user = SaveUser {
            id: id.get(),
            msgs: data.messages,
            last_msg: data.last_message_at,
        };
        let save_user_bytes = save_user.to_raw();
        file.write_all(&save_user_bytes)?;
        hash.update_each(&save_user_bytes);
    }

    // Get the actual number underlying the hash, and add it to the file. This can detect corruption.
    let hash = hash.finish();
    file.write_all(&hash.to_le_bytes())?;

    // Ensure all the data is written to whatever I/O, and not buffered.
    file.flush()?;
    Ok(())
}

pub fn load(file: &mut impl std::io::Read) -> Result<MessageMap, IoError> {
    let mut hash = Fnv1A::new(); // Create a new hash so we can compare them
    let mut messages = MessageMap::new();

    // We need to make sure the first 8 bytes are the same, they are ALWAYS the same in ephemerole
    // save files.
    {
        let mut magic_buf = [0u8; 8]; // Length of our magic bytes

        file.read_exact(&mut magic_buf)?;
        // if the magic bytes aren't the same as the ones we write into every file, this ain't
        // an epd file, bail out with an error
        if magic_buf != MAGIC_BYTES {
            return Err(IoError::new(
                IoErrorKind::InvalidData,
                "Invalid magic for `.epd` file",
            ));
        }
        // if it IS an epd file, it still might be corrupted, so hash the magic bytes
        hash.update_each(&MAGIC_BYTES);
    }

    // Read and hash the length, then convert it to the actual length number
    let len = {
        let mut len_buf = [0u8; 8];
        file.read_exact(&mut len_buf)?;
        hash.update_each(&len_buf);
        u64::from_le_bytes(len_buf)
    };

    // Performance optimization to automatically get the map ready for Many Many Entries.
    // If we have too many entries for the map to contain, we bail out with an error.
    {
        let len_usize = len.try_into().map_err(|_| {
            IoError::new(
                IoErrorKind::Other,
                "You have more then usize::MAX entries??? What??",
            )
        })?;
        messages.try_reserve(len_usize)?;
    }

    // Read a user's data from the save file `len` times
    for _ in 0..len {
        let mut saveuser_buf = [0u8; 24];
        file.read_exact(&mut saveuser_buf)?;
        hash.update_each(&saveuser_buf);

        // Get structured user data from the raw bytes
        let user = SaveUser::from_raw(saveuser_buf);

        // convert the special SaveUser into the mapped data structure
        let user_data = UserData {
            messages: user.msgs,
            last_message_at: user.last_msg,
        };
        // Make sure the user ID isn't 0, that can break things
        let user_id = Id::new_checked(user.id).ok_or_else(|| {
            IoError::new(
                IoErrorKind::InvalidData,
                "Invalid user ID value. Did you tamper with the save?",
            )
        })?;
        // Add the user to the new map
        messages.insert(user_id, user_data);
    }

    // Read out the hash data to a number
    let mut hash_buf = [0u8; 8];
    file.read_exact(&mut hash_buf)?;
    let provided_hash = u64::from_le_bytes(hash_buf);

    // Get the calculated hash and bail if it's invalid
    let real_hash = hash.finish();
    if provided_hash != real_hash {
        return Err(IoError::new(ErrorKind::InvalidData, "Hashes do not match!"));
    }

    Ok(messages)
}

/// A special data structure to encapsulate the storage of each user.
#[derive(Copy, Clone, Debug, Hash)]
struct SaveUser {
    id: u64,
    msgs: u64,
    last_msg: u64,
}

impl SaveUser {
    pub fn to_raw(self) -> [u8; 24] {
        // the output section has this exact size for 3 of this type of number
        let mut out = [0; 24];
        // Copy the ID, message count, and last message timestamp into the output data
        out[0..8].copy_from_slice(self.id.to_le_bytes().as_slice());
        out[8..16].copy_from_slice(self.msgs.to_le_bytes().as_slice());
        out[16..24].copy_from_slice(self.last_msg.to_le_bytes().as_slice());
        out
    }

    pub fn from_raw(data: [u8; 24]) -> Self {
        // These unwraps are okay because we can see (but the compiler can't) that this is a perfect fit.
        // We're taking the raw data and loading it back into numbers in the right order.
        let id = u64::from_le_bytes(data[0..8].try_into().unwrap());
        let msgs = u64::from_le_bytes(data[8..16].try_into().unwrap());
        let last_msg = u64::from_le_bytes(data[16..24].try_into().unwrap());
        Self { id, msgs, last_msg }
    }
}

// A simple checksum implementation of the Fowler-Noll-Vo hash function
// https://en.wikipedia.org/wiki/Fowler%E2%80%93Noll%E2%80%93Vo_hash_function
// Thanks to https://craftinginterpreters.com/hash-tables.html as well
#[derive(Debug)]
pub struct Fnv1A {
    hash: u64,
}

impl Fnv1A {
    const FNV_PRIME: u64 = 1_099_511_628_211;
    const OFFSET_BASIS: u64 = 14_695_981_039_346_656_037;

    pub fn update(&mut self, data: u8) {
        self.hash = self
            .hash
            .bitxor(u64::from(data))
            .wrapping_mul(Self::FNV_PRIME);
    }

    pub fn update_each(&mut self, data: &[u8]) {
        for item in data {
            self.update(*item);
        }
    }

    pub const fn finish(self) -> u64 {
        self.hash
    }

    pub const fn new() -> Self {
        Self {
            hash: Self::OFFSET_BASIS,
        }
    }
}

#[cfg(test)]
mod fnv_tests {
    use super::Fnv1A;

    #[test]
    fn each_agrees() {
        let mut hash1 = Fnv1A::new();
        let mut hash2 = Fnv1A::new();
        hash1.update(1);
        hash1.update(2);
        hash2.update_each(&[1, 2]);
        assert_eq!(hash1.finish(), hash2.finish());
    }

    #[test]
    fn correct_const() {
        assert_eq!(Fnv1A::new().finish(), 0xCBF2_9CE4_8422_2325);
    }

    #[test]
    fn correct_basic() {
        let mut hash = Fnv1A::new();
        hash.update(0x11);
        assert_eq!(hash.finish(), 0xAF63_CC4C_8601_D15C);
    }

    #[test]
    fn correct_nullbyte() {
        let mut hash = Fnv1A::new();
        hash.update(0);
        assert_eq!(hash.finish(), 0xAF63_BD4C_8601_B7DF);
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::Cursor,
        ops::{AddAssign, SubAssign},
    };

    use ephemerole::MessageMap;

    use super::*;

    const fn dummy_data(messages: u64, last_message_at: u64) -> UserData {
        UserData {
            messages,
            last_message_at,
        }
    }

    #[test]
    fn empty_map() {
        let messages = MessageMap::new();
        let mut fake_file = Vec::new();
        save(&messages, &mut Cursor::new(&mut fake_file)).unwrap();
        let new_msgs = load(&mut Cursor::new(&mut fake_file)).unwrap();

        assert_eq!(new_msgs, messages);
    }

    #[test]
    fn one_item() {
        let mut messages = MessageMap::new();
        messages.insert(Id::new(10), dummy_data(128, 241_215));
        let mut fake_file = Vec::new();
        save(&messages, &mut Cursor::new(&mut fake_file)).unwrap();
        let new_msgs = load(&mut Cursor::new(&mut fake_file)).unwrap();

        assert_eq!(new_msgs, messages);
    }

    #[test]
    fn many_items() {
        let mut messages = MessageMap::new();
        for i in 1..1241 {
            messages.insert(Id::new(10 * i), dummy_data(12 * i, 135 * i));
        }
        let mut fake_file = Vec::new();
        save(&messages, &mut Cursor::new(&mut fake_file)).unwrap();
        let new_msgs = load(&mut Cursor::new(&mut fake_file)).unwrap();

        assert_eq!(new_msgs, messages);
    }

    #[test]
    fn checksum_wrong() {
        let mut messages = MessageMap::new();
        for i in 1..1241 {
            messages.insert(Id::new(10 * i), dummy_data(12 * i, 135 * i));
        }
        let mut fake_file = Vec::new();
        save(&messages, &mut Cursor::new(&mut fake_file)).unwrap();
        fake_file.last_mut().unwrap().add_assign(1);
        load(&mut Cursor::new(&mut fake_file)).unwrap_err();
    }

    #[test]
    fn len_too_big() {
        let mut messages = MessageMap::new();
        for i in 1..1241 {
            messages.insert(Id::new(10 * i), dummy_data(12 * i, 135 * i));
        }
        let mut fake_file = Vec::new();
        save(&messages, &mut Cursor::new(&mut fake_file)).unwrap();
        // Add one to the of the len
        fake_file[8..16].copy_from_slice(1242u64.to_le_bytes().as_slice());
        let err = load(&mut Cursor::new(&mut fake_file)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::UnexpectedEof);
    }

    #[test]
    fn len_too_small() {
        let mut messages = MessageMap::new();
        for i in 1..1241 {
            messages.insert(Id::new(10 * i), dummy_data(12 * i, 135 * i));
        }
        let mut fake_file = Vec::new();
        save(&messages, &mut Cursor::new(&mut fake_file)).unwrap();
        // subtract one from the LSB of the length
        fake_file.iter_mut().nth(8).unwrap().sub_assign(1);
        let err = load(&mut Cursor::new(&mut fake_file)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidData);
    }
}
