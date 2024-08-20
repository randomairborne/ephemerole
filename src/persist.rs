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

const MAGIC: u64 = 0x7f8f_58a6_b944_1e85; // A randomly chosen value that identifies .epd files
const MAGIC_BYTES: [u8; 8] = MAGIC.to_le_bytes(); // Byte value of the MAGIC constant, for easier writing

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
    let mut hash = Fnv1A::new();
    let mut messages = MessageMap::new();
    {
        let mut magic_buf = [0u8; 8];

        file.read_exact(&mut magic_buf)?;
        if magic_buf != MAGIC_BYTES {
            return Err(IoError::new(
                IoErrorKind::InvalidData,
                "Invalid magic for `.epd` file",
            ));
        }
        hash.update_each(&MAGIC_BYTES);
    }

    let len = {
        let mut len_buf = [0u8; 8];
        file.read_exact(&mut len_buf)?;
        hash.update_each(&len_buf);
        u64::from_le_bytes(len_buf)
    };

    {
        let len_usize = len.try_into().map_err(|_| {
            IoError::new(
                IoErrorKind::Other,
                "You have more then usize::MAX entries??? What??",
            )
        })?;
        messages.reserve(len_usize);
    }

    for _ in 0..len {
        let mut saveuser_buf = [0u8; 24];
        file.read_exact(&mut saveuser_buf)?;
        hash.update_each(&saveuser_buf);

        let user = SaveUser::from_raw(saveuser_buf);
        let user_data = UserData {
            messages: user.msgs,
            last_message_at: user.last_msg,
        };
        let user_id = Id::new_checked(user.id).ok_or_else(|| {
            IoError::new(
                IoErrorKind::InvalidData,
                "Invalid user ID value. Did you tamper with the save?",
            )
        })?;
        messages.insert(user_id, user_data);
    }

    let mut hash_buf = [0u8; 8];
    file.read_exact(&mut hash_buf)?;
    let provided_hash = u64::from_le_bytes(hash_buf);
    let real_hash = hash.finish();
    if provided_hash != real_hash {
        return Err(IoError::new(ErrorKind::InvalidData, "Hashes do not match!"));
    }

    Ok(messages)
}

#[derive(Copy, Clone, Debug, Hash)]
struct SaveUser {
    id: u64,
    msgs: u64,
    last_msg: u64,
}

impl SaveUser {
    pub fn to_raw(self) -> [u8; 24] {
        let mut out = [0; 24];
        out[0..8].copy_from_slice(self.id.to_le_bytes().as_slice());
        out[8..16].copy_from_slice(self.msgs.to_le_bytes().as_slice());
        out[16..24].copy_from_slice(self.last_msg.to_le_bytes().as_slice());
        out
    }

    pub fn from_raw(data: [u8; 24]) -> Self {
        // These unwraps are okay because we can see (but the compiler can't) that this is a perfect fit.
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
}

impl Fnv1A {
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
        assert_eq!(hash.finish(), 0xaf63_cc4c_8601_d15c);
    }

    #[test]
    fn correct_nullbyte() {
        let mut hash = Fnv1A::new();
        hash.update(0);
        assert_eq!(hash.finish(), 0xaf63_bd4c_8601_b7df);
    }
}

#[cfg(test)]
mod tests {
    use std::{io::Cursor, ops::AddAssign};

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
            messages.insert(Id::new(10), dummy_data(12 * i, 135 * i));
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
            messages.insert(Id::new(10), dummy_data(12 * i, 135 * i));
        }
        let mut fake_file = Vec::new();
        save(&messages, &mut Cursor::new(&mut fake_file)).unwrap();
        fake_file.last_mut().unwrap().add_assign(1);
        load(&mut Cursor::new(&mut fake_file)).unwrap_err();
    }
}
