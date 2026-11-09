//! Thumbnail slabs: append-only files of encoded thumbnails with a positional index.
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicPtr, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Magic header value for Picasa-compatible thumbnail caches (1.60f in LE).
pub const PICASA_MAGIC: u32 = 0x3fcccccd;
pub const INDEX_HEADER_SIZE: usize = 20;
pub const CONTAINER_HEADER_SIZE: usize = 4;
pub const ENTRY_SIZE: usize = 12;
pub const DEFAULT_SHARD_LIMIT: u64 = 2 * 1024 * 1024 * 1024; // 2 GB

pub const CHUNK_SIZE: usize = 65_536;
pub const CHUNK_SHIFT: usize = 16;
pub const CHUNK_MASK: usize = 0xFFFF;
pub const MAX_CHUNKS: usize = 256; // Supports up to 16.7 million file IDs
pub const MAX_CONTAINERS: usize = 256;

/// Compact 12-byte thumbnail index record matching Picasa layout:
/// - `file_index`: target container file number (0 for thumbs_0.db, 1 for thumbs_1.db, etc.)
/// - `version`: orientation algorithm version (honoured on get, rejects stale algorithms)
/// - `offset`: byte offset within container file
/// - `size`: length of thumbnail JPEG blob in bytes (0 if absent)
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct ThumbIndexEntry {
    pub file_index: u16,
    pub version: u16,
    pub offset: u32,
    pub size: u32,
}

impl ThumbIndexEntry {
    pub fn new(file_index: u16, version: u16, offset: u32, size: u32) -> Self {
        Self {
            file_index,
            version,
            offset,
            size,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    pub fn to_bytes(&self) -> [u8; 12] {
        let mut bytes = [0u8; 12];
        bytes[0..2].copy_from_slice(&self.file_index.to_le_bytes());
        bytes[2..4].copy_from_slice(&self.version.to_le_bytes());
        bytes[4..8].copy_from_slice(&self.offset.to_le_bytes());
        bytes[8..12].copy_from_slice(&self.size.to_le_bytes());
        bytes
    }

    pub fn from_bytes(bytes: &[u8; 12]) -> Self {
        let file_index = u16::from_le_bytes(bytes[0..2].try_into().unwrap());
        let version = u16::from_le_bytes(bytes[2..4].try_into().unwrap());
        let offset = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let size = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        Self {
            file_index,
            version,
            offset,
            size,
        }
    }
}

/// Atomic representation of an index entry in memory.
/// Publication metadata is in-memory only; the serialized record remains 12 bytes.
#[repr(C)]
pub struct AtomicIndexEntry {
    sequence: AtomicU64,
    file_index_and_version: AtomicU32,
    offset: AtomicU32,
    size: AtomicU32,
}

impl AtomicIndexEntry {
    pub fn new(file_index: u16, version: u16, offset: u32, size: u32) -> Self {
        let combined = (file_index as u32) | ((version as u32) << 16);
        Self {
            sequence: AtomicU64::new(0),
            file_index_and_version: AtomicU32::new(combined),
            offset: AtomicU32::new(offset),
            size: AtomicU32::new(size),
        }
    }

    pub fn load(&self) -> Option<ThumbIndexEntry> {
        self.load_with(|| {})
    }

    fn load_with(&self, between_fields: impl FnOnce()) -> Option<ThumbIndexEntry> {
        // One bounded attempt: a reader never waits for a paused writer.
        let sequence = self.sequence.load(Ordering::SeqCst);
        if sequence & 1 != 0 {
            return None;
        }
        let combined = self.file_index_and_version.load(Ordering::SeqCst);
        between_fields();
        let offset = self.offset.load(Ordering::SeqCst);
        let size = self.size.load(Ordering::SeqCst);
        if sequence != self.sequence.load(Ordering::SeqCst) || size == 0 {
            return None;
        }
        let file_index = (combined & 0xFFFF) as u16;
        let version = ((combined >> 16) & 0xFFFF) as u16;
        Some(ThumbIndexEntry {
            file_index,
            version,
            offset,
            size,
        })
    }

    pub fn store(&self, entry: ThumbIndexEntry) {
        self.store_with(entry, || {});
    }

    fn store_with(&self, entry: ThumbIndexEntry, between_fields: impl FnOnce()) {
        let sequence = loop {
            let current = self.sequence.load(Ordering::SeqCst);
            if current & 1 == 0 && self.sequence.compare_exchange(
                current, current.wrapping_add(1), Ordering::SeqCst, Ordering::SeqCst,
            ).is_ok() {
                break current;
            }
            std::hint::spin_loop();
        };
        let combined = (entry.file_index as u32) | ((entry.version as u32) << 16);
        self.file_index_and_version.store(combined, Ordering::SeqCst);
        between_fields();
        self.offset.store(entry.offset, Ordering::SeqCst);
        self.size.store(entry.size, Ordering::SeqCst);
        // The SeqCst total order brackets every field access. Equal even reads
        // therefore identify one publication (until a full 64-bit counter wrap).
        self.sequence.store(sequence.wrapping_add(2), Ordering::SeqCst);
    }

    pub fn clear(&self) {
        self.store(ThumbIndexEntry::default());
    }
}

/// Chunked lock-free table providing O(1) pointer-arithmetic lookups across
/// millions of file IDs with zero contention between reader threads.
pub struct SlabIndexTable {
    chunks: [AtomicPtr<AtomicIndexEntry>; MAX_CHUNKS],
}

unsafe impl Send for SlabIndexTable {}
unsafe impl Sync for SlabIndexTable {}

impl SlabIndexTable {
    pub fn new() -> Self {
        const NULL_PTR: AtomicPtr<AtomicIndexEntry> = AtomicPtr::new(std::ptr::null_mut());
        Self {
            chunks: [NULL_PTR; MAX_CHUNKS],
        }
    }

    #[inline]
    pub fn get(&self, file_id: u32) -> Option<ThumbIndexEntry> {
        let id = file_id as usize;
        let chunk_idx = id >> CHUNK_SHIFT;
        if chunk_idx >= MAX_CHUNKS {
            return None;
        }
        let chunk_ptr = self.chunks[chunk_idx].load(Ordering::Acquire);
        if chunk_ptr.is_null() {
            return None;
        }
        let entry_idx = id & CHUNK_MASK;
        let entry = unsafe { &*chunk_ptr.add(entry_idx) };
        entry.load()
    }

    pub fn set(&self, file_id: u32, entry: ThumbIndexEntry) -> Result<(), String> {
        let id = file_id as usize;
        let chunk_idx = id >> CHUNK_SHIFT;
        if chunk_idx >= MAX_CHUNKS {
            return Err(format!("file_id {} exceeds max table capacity", file_id));
        }
        let chunk_ptr = self.ensure_chunk(chunk_idx);
        let entry_idx = id & CHUNK_MASK;
        let slot = unsafe { &*chunk_ptr.add(entry_idx) };
        slot.store(entry);
        Ok(())
    }

    pub fn clear(&self, file_id: u32) {
        let id = file_id as usize;
        let chunk_idx = id >> CHUNK_SHIFT;
        if chunk_idx >= MAX_CHUNKS {
            return;
        }
        let chunk_ptr = self.chunks[chunk_idx].load(Ordering::Acquire);
        if chunk_ptr.is_null() {
            return;
        }
        let entry_idx = id & CHUNK_MASK;
        let slot = unsafe { &*chunk_ptr.add(entry_idx) };
        slot.clear();
    }

    fn ensure_chunk(&self, chunk_idx: usize) -> *mut AtomicIndexEntry {
        let existing = self.chunks[chunk_idx].load(Ordering::Acquire);
        if !existing.is_null() {
            return existing;
        }

        let mut entries = Vec::with_capacity(CHUNK_SIZE);
        for _ in 0..CHUNK_SIZE {
            entries.push(AtomicIndexEntry::new(0, 0, 0, 0));
        }
        let boxed = Box::into_raw(entries.into_boxed_slice()) as *mut AtomicIndexEntry;

        match self.chunks[chunk_idx].compare_exchange(
            std::ptr::null_mut(),
            boxed,
            Ordering::Release,
            Ordering::Acquire,
        ) {
            Ok(_) => boxed,
            Err(already_allocated) => {
                // Another thread allocated this chunk in race
                unsafe {
                    let _ = Box::from_raw(std::slice::from_raw_parts_mut(boxed, CHUNK_SIZE));
                }
                already_allocated
            }
        }
    }
}

impl Drop for SlabIndexTable {
    fn drop(&mut self) {
        for slot in &self.chunks {
            let ptr = slot.swap(std::ptr::null_mut(), Ordering::AcqRel);
            if !ptr.is_null() {
                unsafe {
                    let _ = Box::from_raw(std::slice::from_raw_parts_mut(ptr, CHUNK_SIZE));
                }
            }
        }
    }
}

/// Cross-platform direct offset reader that allows concurrent reads on the same
/// file descriptor across multiple threads without mutating file position and
/// without transaction or read locks.
pub fn file_read_exact_at(file: &File, mut buf: &mut [u8], mut offset: u64) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        while !buf.is_empty() {
            let n = file.seek_read(buf, offset)?;
            if n == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "failed to fill buffer from container",
                ));
            }
            offset += n as u64;
            buf = &mut buf[n..];
        }
        Ok(())
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        file.read_exact_at(buf, offset)
    }
    #[cfg(not(any(windows, unix)))]
    {
        compile_error!("Platform not supported for concurrent offset reads");
    }
}

struct SlabWriterState {
    active_shard: u32,
    active_container_file: File,
    active_container_offset: u64,
    index_file: File,
    max_file_id: u32,
}

/// Sharded slab container thumbnail cache.
///
/// Features:
/// - Append-only flat container slabs (`thumbs_0.db`, `thumbs_1.db`...) sharded at 2GB limits.
/// - 12-byte contiguous index table (`thumbs_index.db`).
/// - Concurrent lock-free reader threads reading via direct offset pointer arithmetic.
/// - Startup in <1ms without B-tree page repair scanning.
pub struct SlabCache {
    base_dir: PathBuf,
    shard_limit: u64,
    index: SlabIndexTable,
    containers: [OnceLock<Arc<File>>; MAX_CONTAINERS],
    writer: Mutex<SlabWriterState>,
}

impl SlabCache {
    /// Open or create a slab cache in the specified directory using the default 2GB shard limit.
    pub fn open(base_dir: impl AsRef<Path>) -> Result<Self, String> {
        Self::open_with_shard_limit(base_dir, DEFAULT_SHARD_LIMIT)
    }

    /// Open or create a slab cache with a configurable shard limit (useful for testing rollover).
    pub fn open_with_shard_limit(base_dir: impl AsRef<Path>, shard_limit: u64) -> Result<Self, String> {
        let base_dir = base_dir.as_ref().to_path_buf();
        if !base_dir.exists() {
            std::fs::create_dir_all(&base_dir).map_err(|e| e.to_string())?;
        }

        // Validate before opening either writer: rejection must not modify the cache.
        let mut active_shard = 0u32;
        while base_dir.join(format!("thumbs_{}.db", active_shard + 1)).exists() {
            active_shard += 1;
            if active_shard as usize >= MAX_CONTAINERS {
                return Err(format!("Container shard {} exceeds supported limit", active_shard));
            }
        }

        const EMPTY_ONCE: OnceLock<Arc<File>> = OnceLock::new();
        let containers = [EMPTY_ONCE; MAX_CONTAINERS];
        let index = SlabIndexTable::new();

        let index_path = base_dir.join("thumbs_index.db");
        let mut index_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&index_path)
            .map_err(|e| format!("Failed to open {}: {}", index_path.display(), e))?;

        let index_len = index_file.metadata().map_err(|e| e.to_string())?.len();
        let mut max_file_id = 0u32;

        if index_len == 0 {
            // Initialize header: magic (4), version (4), count (4), reserved (8)
            let mut header = [0u8; INDEX_HEADER_SIZE];
            header[0..4].copy_from_slice(&PICASA_MAGIC.to_le_bytes());
            header[4..8].copy_from_slice(&1u32.to_le_bytes()); // version
            header[8..12].copy_from_slice(&0u32.to_le_bytes()); // count
            index_file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
            index_file.write_all(&header).map_err(|e| e.to_string())?;
            index_file.flush().map_err(|e| e.to_string())?;
        } else {
            if index_len < INDEX_HEADER_SIZE as u64 {
                return Err(format!("Incomplete index header: {} bytes", index_len));
            }
            // Read and validate header
            let mut header = [0u8; INDEX_HEADER_SIZE];
            index_file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
            index_file.read_exact(&mut header).map_err(|e| e.to_string())?;
            let magic = u32::from_le_bytes(header[0..4].try_into().unwrap());
            if magic != PICASA_MAGIC {
                return Err(format!(
                    "Invalid index magic: expected 0x{:08x}, found 0x{:08x}",
                    PICASA_MAGIC, magic
                ));
            }
            let version = u32::from_le_bytes(header[4..8].try_into().unwrap());
            if version != 1 {
                return Err(format!("Unsupported index version: {}", version));
            }
            let recorded_count = u32::from_le_bytes(header[8..12].try_into().unwrap());

            // Load records into in-memory lock-free table
            let records_len = index_len - INDEX_HEADER_SIZE as u64;
            let total_records = (records_len / ENTRY_SIZE as u64) as usize;
            let mut raw_buf = vec![0u8; records_len as usize];
            index_file.seek(SeekFrom::Start(INDEX_HEADER_SIZE as u64)).map_err(|e| e.to_string())?;
            index_file.read_exact(&mut raw_buf).map_err(|e| e.to_string())?;

            // Inspect each referenced container once. Reject truncated indexed payloads
            // before opening the append writer, leaving rejected cache files unchanged.
            let mut container_lengths = std::collections::BTreeMap::new();
            for i in 0..total_records {
                let offset = i * ENTRY_SIZE;
                let entry_bytes: &[u8; 12] = raw_buf[offset..offset + 12].try_into().unwrap();
                let entry = ThumbIndexEntry::from_bytes(entry_bytes);
                if entry.size > 0 {
                    let container_len = match container_lengths.get(&entry.file_index) {
                        Some(length) => *length,
                        None => {
                            let path = base_dir.join(format!("thumbs_{}.db", entry.file_index));
                            let length = File::open(&path).and_then(|file| file.metadata())
                                .map_err(|e| format!("Failed to inspect {}: {}", path.display(), e))?.len();
                            container_lengths.insert(entry.file_index, length);
                            length
                        }
                    };
                    if u64::from(entry.offset) + u64::from(entry.size) > container_len {
                        return Err(format!("Thumbnail record {} extends beyond container {}",
                            i, entry.file_index));
                    }
                    let file_id = i as u32;
                    index.set(file_id, entry)?;
                    if file_id > max_file_id {
                        max_file_id = file_id;
                    }
                }
            }
            if recorded_count > max_file_id {
                max_file_id = recorded_count;
            }
        }

        let container_path = base_dir.join(format!("thumbs_{}.db", active_shard));
        let mut active_container_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&container_path)
            .map_err(|e| format!("Failed to open {}: {}", container_path.display(), e))?;

        let mut container_len = active_container_file.metadata().map_err(|e| e.to_string())?.len();
        if container_len < CONTAINER_HEADER_SIZE as u64 {
            let magic_bytes = PICASA_MAGIC.to_le_bytes();
            active_container_file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
            active_container_file.write_all(&magic_bytes).map_err(|e| e.to_string())?;
            active_container_file.flush().map_err(|e| e.to_string())?;
            container_len = CONTAINER_HEADER_SIZE as u64;
        }

        // Cache read handle for active container
        let read_handle = OpenOptions::new()
            .read(true)
            .open(&container_path)
            .map_err(|e| e.to_string())?;
        containers[active_shard as usize].get_or_init(|| Arc::new(read_handle));

        let writer = Mutex::new(SlabWriterState {
            active_shard,
            active_container_file,
            active_container_offset: container_len,
            index_file,
            max_file_id,
        });

        Ok(Self {
            base_dir,
            shard_limit,
            index,
            containers,
            writer,
        })
    }

    /// Read thumbnail bytes for `file_id`.
    ///
    /// CONCURRENCY GUARANTEE:
    /// This method is lock-free and thread-safe. Multiple readers read concurrently
    /// via direct offset without acquiring transaction or write locks.
    ///
    /// ORIENTATION GUARANTEE:
    /// Only returns thumbnail bytes if the cached orientation version matches the
    /// current ORIENTATION_ALGORITHM_VERSION. Stale or legacy uncertified thumbnails
    /// return None to trigger regeneration.
    pub fn get(&self, file_id: i64) -> Option<Vec<u8>> {
        if file_id <= 0 {
            return None;
        }
        let entry = self.index.get(file_id as u32)?;
        if entry.size == 0 {
            return None;
        }
        if entry.version != crate::t_image::ORIENTATION_ALGORITHM_VERSION as u16 {
            return None;
        }

        let file = self.get_container_file(entry.file_index as u32).ok()?;
        let mut buf = vec![0u8; entry.size as usize];
        file_read_exact_at(&file, &mut buf, entry.offset as u64).ok()?;
        Some(buf)
    }

    /// Read raw thumbnail bytes for `file_id` without orientation version verification.
    pub fn get_raw(&self, file_id: i64) -> Option<Vec<u8>> {
        if file_id <= 0 {
            return None;
        }
        let entry = self.index.get(file_id as u32)?;
        if entry.size == 0 {
            return None;
        }

        let file = self.get_container_file(entry.file_index as u32).ok()?;
        let mut buf = vec![0u8; entry.size as usize];
        file_read_exact_at(&file, &mut buf, entry.offset as u64).ok()?;
        Some(buf)
    }

    /// Return the orientation version recorded for `file_id`.
    pub fn get_version(&self, file_id: i64) -> Option<u16> {
        if file_id <= 0 {
            return None;
        }
        self.index.get(file_id as u32).map(|e| e.version)
    }

    /// Check if thumbnail for `file_id` is present in the slab cache.
    #[inline]
    pub fn contains(&self, file_id: i64) -> bool {
        if file_id <= 0 {
            return false;
        }
        self.index.get(file_id as u32).is_some_and(|e| e.size > 0 && e.version == crate::t_image::ORIENTATION_ALGORITHM_VERSION as u16)
    }

    /// Retrieve the raw index entry for `file_id`.
    pub fn get_entry(&self, file_id: i64) -> Option<ThumbIndexEntry> {
        if file_id <= 0 {
            return None;
        }
        self.index.get(file_id as u32)
    }

    /// Store thumbnail bytes for `file_id` with current ORIENTATION_ALGORITHM_VERSION.
    ///
    /// Writers append to the active container slab and atomically update the
    /// index table. Readers are never blocked by this operation.
    pub fn put(&self, file_id: i64, data: &[u8]) -> Result<(), String> {
        self.put_with_version(
            file_id,
            data,
            crate::t_image::ORIENTATION_ALGORITHM_VERSION as u16,
        )
    }

    /// Store thumbnail bytes for `file_id` with an explicit orientation algorithm version.
    pub fn put_with_version(&self, file_id: i64, data: &[u8], version: u16) -> Result<(), String> {
        if file_id <= 0 || data.is_empty() {
            return Ok(());
        }

        let mut w = self.writer.lock().map_err(|e| e.to_string())?;

        // Check if append would exceed shard limit
        let payload_len = data.len() as u64;
        if w.active_container_offset + payload_len > self.shard_limit {
            // Rollover to next shard
            let next_shard = w.active_shard + 1;
            if next_shard as usize >= MAX_CONTAINERS {
                return Err("Maximum container shards reached".to_string());
            }
            let next_path = self.base_dir.join(format!("thumbs_{}.db", next_shard));
            let mut next_file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .open(&next_path)
                .map_err(|e| format!("Failed to create {}: {}", next_path.display(), e))?;

            next_file
                .write_all(&PICASA_MAGIC.to_le_bytes())
                .map_err(|e| e.to_string())?;
            next_file.flush().map_err(|e| e.to_string())?;

            w.active_shard = next_shard;
            w.active_container_file = next_file;
            w.active_container_offset = CONTAINER_HEADER_SIZE as u64;

            // Open read handle for readers
            let read_handle = OpenOptions::new()
                .read(true)
                .open(&next_path)
                .map_err(|e| e.to_string())?;
            self.containers[next_shard as usize].get_or_init(|| Arc::new(read_handle));
        }

        let shard = w.active_shard as u16;
        let offset = w.active_container_offset as u32;
        let size = data.len() as u32;

        // 1. Append payload to container file
        w.active_container_file
            .seek(SeekFrom::Start(offset as u64))
            .map_err(|e| e.to_string())?;
        w.active_container_file.write_all(data).map_err(|e| e.to_string())?;
        w.active_container_offset += payload_len;

        // 2. Persist record to index file
        let entry = ThumbIndexEntry::new(shard, version, offset, size);
        let file_pos = INDEX_HEADER_SIZE as u64 + (file_id as u64) * ENTRY_SIZE as u64;
        w.index_file
            .seek(SeekFrom::Start(file_pos))
            .map_err(|e| e.to_string())?;
        w.index_file.write_all(&entry.to_bytes()).map_err(|e| e.to_string())?;

        // 3. Update count header if file_id exceeds current max
        let id_u32 = file_id as u32;
        if id_u32 > w.max_file_id {
            w.max_file_id = id_u32;
            w.index_file
                .seek(SeekFrom::Start(8))
                .map_err(|e| e.to_string())?;
            w.index_file
                .write_all(&id_u32.to_le_bytes())
                .map_err(|e| e.to_string())?;
        }

        // 4. Update in-memory lock-free table (readers see update atomically)
        self.index.set(id_u32, entry)?;

        Ok(())
    }

    /// Invalidate/remove thumbnail for `file_id`.
    pub fn remove(&self, file_id: i64) -> Result<(), String> {
        if file_id <= 0 {
            return Ok(());
        }
        let id_u32 = file_id as u32;
        let mut w = self.writer.lock().map_err(|e| e.to_string())?;

        // Zero out in index file
        let file_pos = INDEX_HEADER_SIZE as u64 + (file_id as u64) * ENTRY_SIZE as u64;
        w.index_file
            .seek(SeekFrom::Start(file_pos))
            .map_err(|e| e.to_string())?;
        w.index_file.write_all(&[0u8; ENTRY_SIZE]).map_err(|e| e.to_string())?;

        // Clear in in-memory table
        self.index.clear(id_u32);

        Ok(())
    }

    /// Flush all pending writes and buffers to disk.
    pub fn flush(&self) -> Result<(), String> {
        let mut w = self.writer.lock().map_err(|e| e.to_string())?;
        w.active_container_file.flush().map_err(|e| e.to_string())?;
        w.index_file.flush().map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Return the active container shard index.
    pub fn active_shard(&self) -> u32 {
        self.writer.lock().map(|w| w.active_shard).unwrap_or(0)
    }

    /// Return the maximum file_id recorded in the index.
    pub fn count(&self) -> u32 {
        self.writer.lock().map(|w| w.max_file_id).unwrap_or(0)
    }

    /// Return the number of records represented by the max file_id.
    pub fn len(&self) -> usize {
        self.count() as usize
    }

    /// Return true if the cache contains no records.
    pub fn is_empty(&self) -> bool {
        self.count() == 0
    }

    /// Retrieve or lazily open a read handle for container shard `shard`.
    fn get_container_file(&self, shard: u32) -> Result<Arc<File>, String> {
        let idx = shard as usize;
        if idx >= MAX_CONTAINERS {
            return Err("Container shard index out of bounds".to_string());
        }

        if let Some(file) = self.containers[idx].get() {
            return Ok(Arc::clone(file));
        }

        let path = self.base_dir.join(format!("thumbs_{}.db", shard));
        let file = OpenOptions::new()
            .read(true)
            .open(&path)
            .map_err(|e| format!("Failed to open container {}: {}", path.display(), e))?;

        let arc = Arc::new(file);
        let _ = self.containers[idx].set(Arc::clone(&arc));
        Ok(self.containers[idx].get().cloned().unwrap_or(arc))
    }

    /// Migrate thumbnails from an existing redb database file into this slab cache.
    /// Preserves the orientation algorithm version recorded in redb.
    /// Returns the number of migrated thumbnails.
    pub fn migrate_from_redb(&self, redb_path: &Path) -> Result<usize, String> {
        if !redb_path.exists() {
            return Ok(0);
        }

        let db = redb::Database::open(redb_path).map_err(|e| e.to_string())?;
        let read_tx = db.begin_read().map_err(|e| e.to_string())?;

        let table_def: redb::TableDefinition<i64, &[u8]> = redb::TableDefinition::new("thumbnails");
        let version_def: redb::TableDefinition<i64, u32> = redb::TableDefinition::new("thumbnail_orientation_versions");
        let table = match read_tx.open_table(table_def) {
            Ok(t) => t,
            Err(_) => return Ok(0),
        };
        let versions_table = read_tx.open_table(version_def).ok();

        use redb::ReadableTable;
        let mut count = 0;
        let mut w = self.writer.lock().map_err(|e| e.to_string())?;

        for item in table.iter().map_err(|e| e.to_string())? {
            let (key, value) = item.map_err(|e| e.to_string())?;
            let file_id = key.value();
            let data = value.value();
            if file_id <= 0 || data.is_empty() {
                continue;
            }
            let version = versions_table
                .as_ref()
                .and_then(|vt| vt.get(file_id).ok().flatten())
                .map(|v| v.value() as u16)
                .unwrap_or(0);

            let payload_len = data.len() as u64;
            if w.active_container_offset + payload_len > self.shard_limit {
                let next_shard = w.active_shard + 1;
                if next_shard as usize >= MAX_CONTAINERS {
                    return Err("Maximum container shards reached".to_string());
                }
                let next_path = self.base_dir.join(format!("thumbs_{}.db", next_shard));
                let mut next_file = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .open(&next_path)
                    .map_err(|e| format!("Failed to create {}: {}", next_path.display(), e))?;

                next_file
                    .write_all(&PICASA_MAGIC.to_le_bytes())
                    .map_err(|e| e.to_string())?;
                next_file.flush().map_err(|e| e.to_string())?;

                w.active_shard = next_shard;
                w.active_container_file = next_file;
                w.active_container_offset = CONTAINER_HEADER_SIZE as u64;

                let read_handle = OpenOptions::new()
                    .read(true)
                    .open(&next_path)
                    .map_err(|e| e.to_string())?;
                self.containers[next_shard as usize].get_or_init(|| Arc::new(read_handle));
            }

            let shard = w.active_shard as u16;
            let offset = w.active_container_offset as u32;
            let size = data.len() as u32;

            w.active_container_file
                .seek(SeekFrom::Start(offset as u64))
                .map_err(|e| e.to_string())?;
            w.active_container_file.write_all(data).map_err(|e| e.to_string())?;
            w.active_container_offset += payload_len;

            let entry = ThumbIndexEntry::new(shard, version, offset, size);
            let file_pos = INDEX_HEADER_SIZE as u64 + (file_id as u64) * ENTRY_SIZE as u64;
            w.index_file
                .seek(SeekFrom::Start(file_pos))
                .map_err(|e| e.to_string())?;
            w.index_file.write_all(&entry.to_bytes()).map_err(|e| e.to_string())?;

            let id_u32 = file_id as u32;
            if id_u32 > w.max_file_id {
                w.max_file_id = id_u32;
            }

            self.index.set(id_u32, entry)?;
            count += 1;
        }

        let max_id = w.max_file_id;
        w.index_file
            .seek(SeekFrom::Start(8))
            .map_err(|e| e.to_string())?;
        w.index_file
            .write_all(&max_id.to_le_bytes())
            .map_err(|e| e.to_string())?;

        w.active_container_file.flush().map_err(|e| e.to_string())?;
        w.index_file.flush().map_err(|e| e.to_string())?;

        drop(w);
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::time::Instant;

    #[test]
    fn test_thumb_index_entry_layout() {
        assert_eq!(std::mem::size_of::<ThumbIndexEntry>(), 12);

        let entry = ThumbIndexEntry::new(2, crate::t_image::ORIENTATION_ALGORITHM_VERSION as u16, 4096, 65536);
        let bytes = entry.to_bytes();
        let decoded = ThumbIndexEntry::from_bytes(&bytes);
        assert_eq!(entry, decoded);
        let atomic = AtomicIndexEntry::new(entry.file_index, entry.version, entry.offset, entry.size);
        assert_eq!(atomic.load(), Some(decoded));
    }

    #[test]
    fn test_slab_cache_basic_operations() {
        let temp_dir = std::env::temp_dir().join(format!("lap_slab_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);

        {
            let cache = SlabCache::open(&temp_dir).unwrap();
            assert_eq!(cache.get(1), None);
            assert!(!cache.contains(1));

            let sample_data = b"\xFF\xD8\xFF\xE0\x00\x10JFIF...sample_jpeg_bytes...\xFF\xD9";
            cache.put(1, sample_data).unwrap();
            cache.put(2, b"second_photo_thumb").unwrap();

            assert!(cache.contains(1));
            assert!(cache.contains(2));
            assert!(!cache.contains(3));

            assert_eq!(cache.get(1).unwrap(), sample_data);
            assert_eq!(cache.get(2).unwrap(), b"second_photo_thumb");

            cache.remove(1).unwrap();
            assert_eq!(cache.get(1), None);
            assert!(!cache.contains(1));
            assert_eq!(cache.get(2).unwrap(), b"second_photo_thumb");
        }

        // Reopen cache and verify persistence
        {
            let cache = SlabCache::open(&temp_dir).unwrap();
            assert_eq!(cache.get(1), None);
            assert_eq!(cache.get(2).unwrap(), b"second_photo_thumb");
        }

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_orientation_version_honoured() {
        let temp_dir = std::env::temp_dir().join(format!("lap_orient_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);

        let cache = SlabCache::open(&temp_dir).unwrap();
        let sample = b"orientation_test_payload";

        // Write with current version
        cache.put(1, sample).unwrap();
        assert_eq!(cache.get(1).unwrap(), sample);
        assert_eq!(cache.get_raw(1).unwrap(), sample);
        assert_eq!(cache.get_version(1), Some(crate::t_image::ORIENTATION_ALGORITHM_VERSION as u16));
        assert!(cache.contains(1));

        // Write with outdated/stale version (e.g. 0)
        cache.put_with_version(2, sample, 0).unwrap();
        // get() MUST reject stale version
        assert_eq!(cache.get(2), None);
        // get_raw() retrieves the bytes for offline migration/inspection
        assert_eq!(cache.get_raw(2).unwrap(), sample);
        assert_eq!(cache.get_version(2), Some(0));
        assert!(!cache.contains(2));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_shard_rollover() {
        let temp_dir = std::env::temp_dir().join(format!("lap_shard_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);

        // Set small shard limit of 100 bytes to force shard rollover
        let small_limit = 100u64;
        let cache = SlabCache::open_with_shard_limit(&temp_dir, small_limit).unwrap();

        let data1 = vec![1u8; 60];
        let data2 = vec![2u8; 60]; // This will exceed 100 bytes and trigger thumbs_1.db
        let data3 = vec![3u8; 60]; // Trigger thumbs_2.db

        cache.put(1, &data1).unwrap();
        assert_eq!(cache.active_shard(), 0);

        cache.put(2, &data2).unwrap();
        assert_eq!(cache.active_shard(), 1);

        cache.put(3, &data3).unwrap();
        assert_eq!(cache.active_shard(), 2);

        // Verify direct reads from across shards
        assert_eq!(cache.get(1).unwrap(), data1);
        assert_eq!(cache.get(2).unwrap(), data2);
        assert_eq!(cache.get(3).unwrap(), data3);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_concurrent_lock_free_reads() {
        let temp_dir = std::env::temp_dir().join(format!("lap_concurrent_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);

        let cache = Arc::new(SlabCache::open(&temp_dir).unwrap());

        // Prepopulate 1,000 thumbnails
        for i in 1..=1000 {
            let thumb = format!("thumbnail_content_{}", i).into_bytes();
            cache.put(i, &thumb).unwrap();
        }

        // Spawn 8 reader threads reading concurrently without locks
        let running = Arc::new(AtomicBool::new(true));
        let (read_ack, read_started) = std::sync::mpsc::channel();
        let mut handles = Vec::new();

        for _ in 0..8 {
            let c = Arc::clone(&cache);
            let r = Arc::clone(&running);
            let ack = read_ack.clone();
            handles.push(std::thread::spawn(move || {
                let mut reads = 0;
                while r.load(Ordering::Relaxed) && reads < 5000 {
                    let id = (reads % 1000) + 1;
                    let thumb = c.get(id).expect("thumbnail should exist");
                    assert!(thumb.starts_with(b"thumbnail_content_"));
                    reads += 1;
                    if reads == 1 {
                        ack.send(()).unwrap();
                    }
                }
                reads
            }));
        }

        // Every reader must actually read before the writer can shut it down.
        drop(read_ack);
        for _ in 0..8 {
            read_started.recv_timeout(std::time::Duration::from_secs(30)).unwrap();
        }

        // While threads are reading, concurrently write new thumbnails
        for i in 1001..=1200 {
            let thumb = format!("live_written_{}", i).into_bytes();
            cache.put(i, &thumb).unwrap();
        }

        running.store(false, Ordering::Relaxed);
        for h in handles {
            let count = h.join().unwrap();
            assert!(count > 0);
        }

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn benchmark_10k_reads_under_50ms() {
        let temp_dir = std::env::temp_dir().join(format!("lap_bench_10k_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);

        let cache = SlabCache::open(&temp_dir).unwrap();

        // 1. Populate 10,000 thumbnails
        let sample_jpeg = b"\xFF\xD8\xFF\xE0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00\xFF\xDB\x00\x43\x00thumbnail_data_blob_simulating_jpeg_image_content_here\xFF\xD9";
        for id in 1..=10_000 {
            cache.put(id, sample_jpeg).unwrap();
        }
        cache.flush().unwrap();

        // 2. Measure cache reopening (startup latency)
        let open_start = Instant::now();
        let reopened_cache = SlabCache::open(&temp_dir).unwrap();
        let open_duration = open_start.elapsed();
        println!("Cache startup/reopen duration: {:?}", open_duration);
        assert!(
            open_duration.as_millis() < 50,
            "Cache startup took {:?}, exceeding 50ms requirement",
            open_duration
        );

        // 3. Benchmark: 10,000 thumbnail read operations
        let read_start = Instant::now();
        let mut total_bytes = 0usize;
        for id in 1..=10_000 {
            let data = reopened_cache.get(id).expect("thumbnail must exist");
            total_bytes += data.len();
        }
        let read_duration = read_start.elapsed();
        println!(
            "10,000 thumbnail read operations took: {:?} (total bytes: {})",
            read_duration, total_bytes
        );

        assert!(
            read_duration.as_millis() < 50,
            "Benchmark failed: 10,000 reads took {:?}, must be <50ms",
            read_duration
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    fn isolated_dir(label: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("lap112_{}_{}_{}", label, std::process::id(), nonce));
        std::fs::create_dir(&path).unwrap();
        path
    }

    fn directory_snapshot(path: &Path) -> std::collections::BTreeMap<std::ffi::OsString, (u64, Vec<u8>)> {
        std::fs::read_dir(path).unwrap().map(|entry| {
            let entry = entry.unwrap();
            // Query the open file, not potentially stale Windows directory metadata.
            let mut file = File::open(entry.path()).unwrap();
            let length = file.metadata().unwrap().len();
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).unwrap();
            (entry.file_name(), (length, bytes))
        }).collect()
    }

    #[test]
    fn test_valid_reopen_append_control() {
        let dir = isolated_dir("lap113_control");
        {
            let cache = SlabCache::open(&dir).unwrap();
            cache.put(1, b"AAAA").unwrap();
            cache.put(2, b"BBBB").unwrap();
            cache.flush().unwrap();
        }
        {
            let cache = SlabCache::open(&dir).unwrap();
            cache.put(3, b"CCCC").unwrap();
            cache.flush().unwrap();
            for (id, expected) in [(1, b"AAAA"), (2, b"BBBB"), (3, b"CCCC")] {
                assert_eq!(cache.get(id).as_deref(), Some(expected.as_slice()));
                assert_eq!(cache.get_raw(id).as_deref(), Some(expected.as_slice()));
            }
        }
        let cache = SlabCache::open(&dir).unwrap();
        for (id, expected) in [(1, b"AAAA"), (2, b"BBBB"), (3, b"CCCC")] {
            assert_eq!(cache.get(id).as_deref(), Some(expected.as_slice()));
            assert_eq!(cache.get_raw(id).as_deref(), Some(expected.as_slice()));
        }
        drop(cache);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_damaged_payload_does_not_resurrect_after_append_and_reopen() {
        let dir = isolated_dir("lap113_truncated");
        eprintln!("Truncation fixture: {}", dir.display());
        {
            let cache = SlabCache::open(&dir).unwrap();
            cache.put(1, b"AAAA").unwrap();
            cache.put(2, b"BBBB").unwrap();
            cache.flush().unwrap();
        }
        let file = OpenOptions::new().write(true).open(dir.join("thumbs_0.db")).unwrap();
        file.set_len(10).unwrap();
        drop(file);
        let before = directory_snapshot(&dir);
        let cache = match SlabCache::open(&dir) {
            Ok(cache) => cache,
            Err(_) => {
                // Rejecting the damaged cache unchanged is an allowed policy.
                assert_eq!(directory_snapshot(&dir), before);
                assert!(SlabCache::open(&dir).is_err());
                assert_eq!(directory_snapshot(&dir), before);
                std::fs::remove_dir_all(&dir).unwrap();
                return;
            }
        };
        assert_eq!(cache.get(1).as_deref(), Some(b"AAAA".as_slice()));
        assert!(cache.get(2).is_none());
        assert!(cache.get_raw(2).is_none());
        cache.put(3, b"CCCC").unwrap();
        cache.flush().unwrap();
        assert_eq!(cache.get(1).as_deref(), Some(b"AAAA".as_slice()));
        assert_eq!(cache.get(3).as_deref(), Some(b"CCCC".as_slice()));
        let after_append = (cache.get(2), cache.get_raw(2));
        drop(cache);
        let cache = SlabCache::open(&dir).unwrap();
        assert_eq!(cache.get(1).as_deref(), Some(b"AAAA".as_slice()));
        assert_eq!(cache.get(3).as_deref(), Some(b"CCCC".as_slice()));
        let after_reopen = (cache.get(2), cache.get_raw(2));
        assert!(after_append.0.is_none() && after_append.1.is_none()
            && after_reopen.0.is_none() && after_reopen.1.is_none(),
            "Damaged ID resurrected: after append {after_append:?}, after reopen {after_reopen:?}");
        drop(cache);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_atomic_writer_overlap_is_a_bounded_miss() {
        let a = ThumbIndexEntry::new(1, 1, 111, 11);
        let b = ThumbIndexEntry::new(2, 2, 222, 22);
        let slot = Arc::new(AtomicIndexEntry::new(a.file_index, a.version, a.offset, a.size));
        assert_eq!(slot.load(), Some(a));
        let (paused_tx, paused_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let writer_slot = Arc::clone(&slot);
        let writer = std::thread::spawn(move || {
            writer_slot.store_with(b, || {
                paused_tx.send(()).unwrap();
                let _ = resume_rx.recv();
            });
        });
        paused_rx.recv_timeout(std::time::Duration::from_secs(30)).unwrap();
        let (read_tx, read_rx) = std::sync::mpsc::channel();
        let reader_slot = Arc::clone(&slot);
        let reader = std::thread::spawn(move || read_tx.send(reader_slot.load()).unwrap());
        let observed = read_rx.recv_timeout(std::time::Duration::from_secs(30));
        // Release before asserting: a wrongly spinning reader must fail, not hang cleanup.
        resume_tx.send(()).unwrap();
        writer.join().unwrap();
        reader.join().unwrap();
        assert_eq!(observed.unwrap(), None);
        assert_eq!(slot.load(), Some(b));
        slot.clear();
        assert_eq!(slot.load(), None);
        slot.store(a);
        assert_eq!(slot.load(), Some(a));
    }

    #[test]
    fn test_atomic_reader_overlap_rejects_mixed_fields() {
        let a = ThumbIndexEntry::new(1, 1, 111, 11);
        let b = ThumbIndexEntry::new(2, 2, 222, 22);
        let slot = Arc::new(AtomicIndexEntry::new(a.file_index, a.version, a.offset, a.size));
        let (paused_tx, paused_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let reader_slot = Arc::clone(&slot);
        let reader = std::thread::spawn(move || reader_slot.load_with(|| {
            paused_tx.send(()).unwrap();
            let _ = resume_rx.recv();
        }));
        paused_rx.recv_timeout(std::time::Duration::from_secs(30)).unwrap();
        slot.store(b);
        resume_tx.send(()).unwrap();
        assert_eq!(reader.join().unwrap(), None);
        assert_eq!(slot.load(), Some(b));
    }

    #[test]
    fn test_atomic_publication_counter_wrap() {
        let slot = AtomicIndexEntry::new(1, 1, 111, 11);
        slot.sequence.store(u64::MAX - 1, Ordering::SeqCst);
        let b = ThumbIndexEntry::new(2, 2, 222, 22);
        slot.store(b);
        assert_eq!(slot.sequence.load(Ordering::SeqCst), 0);
        assert_eq!(slot.load(), Some(b));
    }

    #[test]
    fn test_replacement_and_remove_payload_reads() {
        let dir = isolated_dir("payloads");
        let cache = Arc::new(SlabCache::open(&dir).unwrap());
        let a = vec![0xA1; 11];
        let b = vec![0xB2; 22];
        cache.put(1, &a).unwrap();
        let running = Arc::new(AtomicBool::new(true));
        let (read_ack, read_started) = std::sync::mpsc::channel();
        let mut readers = Vec::new();
        for _ in 0..4 {
            let cache = Arc::clone(&cache);
            let running = Arc::clone(&running);
            let ack = read_ack.clone();
            let a = a.clone();
            let b = b.clone();
            readers.push(std::thread::spawn(move || {
                let mut reads = 0;
                while running.load(Ordering::SeqCst) {
                    if let Some(bytes) = cache.get(1) {
                        assert!(bytes == a || bytes == b, "hybrid payload: {:?}", bytes);
                    }
                    reads += 1;
                    if reads == 1 { ack.send(()).unwrap(); }
                }
                reads
            }));
        }
        drop(read_ack);
        for _ in 0..4 {
            read_started.recv_timeout(std::time::Duration::from_secs(30)).unwrap();
        }
        for _ in 0..500 {
            cache.put(1, &b).unwrap();
            cache.remove(1).unwrap();
            cache.put(1, &a).unwrap();
        }
        running.store(false, Ordering::SeqCst);
        for reader in readers { assert!(reader.join().unwrap() > 0); }
        assert_eq!(cache.get(1), Some(a.clone()));
        drop(cache);
        let reopened = SlabCache::open(&dir).unwrap();
        assert_eq!(reopened.get(1), Some(a));
        drop(reopened);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn test_257_shards_rejected_without_changes_or_panic() {
        let dir = isolated_dir("257_shards");
        for shard in 0..=MAX_CONTAINERS {
            std::fs::write(dir.join(format!("thumbs_{}.db", shard)), PICASA_MAGIC.to_le_bytes()).unwrap();
        }
        let before = directory_snapshot(&dir);
        let result = std::panic::catch_unwind(|| SlabCache::open(&dir));
        assert_eq!(directory_snapshot(&dir), before);
        assert!(result.is_ok(), "open panicked at shard256");
        assert!(result.unwrap().is_err());
        assert!(!dir.join("thumbs_index.db").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn test_256_shards_open_but_rollover_is_rejected() {
        let dir = isolated_dir("256_shards");
        for shard in 0..MAX_CONTAINERS {
            std::fs::write(dir.join(format!("thumbs_{}.db", shard)), PICASA_MAGIC.to_le_bytes()).unwrap();
        }
        let cache = SlabCache::open_with_shard_limit(&dir, 4).unwrap();
        assert_eq!(cache.active_shard(), 255);
        let before = directory_snapshot(&dir);
        assert!(cache.put(1, b"A").is_err());
        assert_eq!(directory_snapshot(&dir), before);
        assert!(!dir.join("thumbs_256.db").exists());
        drop(cache);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn test_unknown_index_version_preserved_and_rejected() {
        let dir = isolated_dir("version99");
        let mut header = [0u8; INDEX_HEADER_SIZE];
        header[0..4].copy_from_slice(&PICASA_MAGIC.to_le_bytes());
        header[4..8].copy_from_slice(&99u32.to_le_bytes());
        std::fs::write(dir.join("thumbs_index.db"), header).unwrap();
        let before = directory_snapshot(&dir);
        let error = SlabCache::open(&dir).err().expect("version99 must be rejected");
        assert!(error.contains("version") && error.contains("99"), "{}", error);
        assert_eq!(directory_snapshot(&dir), before);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn test_nonempty_short_index_preserved_and_rejected() {
        let dir = isolated_dir("short_index");
        std::fs::write(dir.join("thumbs_index.db"), [0xAB; 7]).unwrap();
        let before = directory_snapshot(&dir);
        assert!(SlabCache::open(&dir).is_err());
        assert_eq!(directory_snapshot(&dir), before);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn test_empty_index_initializes() {
        let dir = isolated_dir("empty_index");
        std::fs::write(dir.join("thumbs_index.db"), []).unwrap();
        let cache = SlabCache::open(&dir).unwrap();
        let header = std::fs::read(dir.join("thumbs_index.db")).unwrap();
        assert_eq!(header.len(), INDEX_HEADER_SIZE);
        assert_eq!(&header[0..4], &PICASA_MAGIC.to_le_bytes());
        assert_eq!(&header[4..8], &1u32.to_le_bytes());
        cache.put(1, b"new index payload").unwrap();
        assert_eq!(cache.get(1).unwrap(), b"new index payload");
        drop(cache);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn test_redb_migration() {
        let temp_dir = std::env::temp_dir().join(format!("lap_migrate_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        let _ = std::fs::create_dir_all(&temp_dir);

        let redb_path = temp_dir.join("legacy_thumbs.redb");

        // Create legacy redb database with 50 entries
        {
            let db = redb::Database::create(&redb_path).unwrap();
            let write_tx = db.begin_write().unwrap();
            {
                let table_def: redb::TableDefinition<i64, &[u8]> = redb::TableDefinition::new("thumbnails");
                let mut table = write_tx.open_table(table_def).unwrap();
                for id in 1..=50 {
                    let data = format!("redb_legacy_thumb_{}", id).into_bytes();
                    table.insert(id, data.as_slice()).unwrap();
                }
            }
            {
                let version_def: redb::TableDefinition<i64, u32> = redb::TableDefinition::new("thumbnail_orientation_versions");
                let mut v_table = write_tx.open_table(version_def).unwrap();
                for id in 1..=50 {
                    v_table.insert(id, crate::t_image::ORIENTATION_ALGORITHM_VERSION).unwrap();
                }
            }
            write_tx.commit().unwrap();
        }

        // Migrate into slab cache
        let slab_dir = temp_dir.join("slabs");
        let slab_cache = SlabCache::open(&slab_dir).unwrap();
        let migrated = slab_cache.migrate_from_redb(&redb_path).unwrap();
        assert_eq!(migrated, 50);

        for id in 1..=50 {
            let data = slab_cache.get(id).unwrap();
            assert_eq!(data, format!("redb_legacy_thumb_{}", id).into_bytes());
        }

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
