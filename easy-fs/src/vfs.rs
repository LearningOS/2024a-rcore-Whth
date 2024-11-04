use super::{
    block_cache_sync_all, get_block_cache, BlockDevice, DirEntry, DiskInode, DiskInodeType,
    EasyFileSystem, DIRENT_SZ,
};
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use spin::{Mutex, MutexGuard};
/// Virtual filesystem layer over easy-fs
pub struct Inode {
    block_id: usize,
    block_offset: usize,
    fs: Arc<Mutex<EasyFileSystem>>,
    block_device: Arc<dyn BlockDevice>,
}

impl Inode {
    /// Create a vfs inode
    pub fn new(
        block_id: u32,
        block_offset: usize,
        fs: Arc<Mutex<EasyFileSystem>>,
        block_device: Arc<dyn BlockDevice>,
    ) -> Self {
        Self {
            block_id: block_id as usize,
            block_offset,
            fs,
            block_device,
        }
    }

    /// Clear the inode

    pub fn add_ref_count(&self) {
        self.modify_disk_inode(|inode| inode.add_ref_count());
        block_cache_sync_all();
    }

    /// Decrease the ref count of the inode

    pub fn sub_ref_count(&self) {
        self.modify_disk_inode(|inode| inode.sub_ref_count());
        block_cache_sync_all();
        if self.disk_inode_ref_count() == 0 {
            self.clear()
        }
    }
    /// Get the inode id
    pub fn disk_inode_id(&self) -> u32 {
        self.read_disk_inode(|dinode| {
            dinode.inode_id()
        })
    }


    /// Get the ref count of the inode
    pub fn disk_inode_ref_count(&self) -> u32 {
        self.read_disk_inode(|dinode| {
            dinode.ref_count()
        })
    }


    /// Check whether it is a directory
    pub fn is_dir(&self) -> bool {
        self.read_disk_inode(|inode| inode.is_dir())
    }

    /// Check whether it is a file
    pub fn is_file(&self) -> bool {
        self.read_disk_inode(|inode| inode.is_file())
    }

    /// Call a function over a disk inode to read it
    fn read_disk_inode<V>(&self, f: impl FnOnce(&DiskInode) -> V) -> V {
        get_block_cache(self.block_id, Arc::clone(&self.block_device))
            .lock()
            .read(self.block_offset, f)
    }
    /// Call a function over a disk inode to modify it
    fn modify_disk_inode<V>(&self, f: impl FnOnce(&mut DiskInode) -> V) -> V {
        get_block_cache(self.block_id, Arc::clone(&self.block_device))
            .lock()
            .modify(self.block_offset, f)
    }
    /// Find inode under a disk inode by name
    fn find_inode_id(&self, name: &str, disk_inode: &DiskInode) -> Option<u32> {
        // assert it is a directory
        assert!(disk_inode.is_dir());
        disk_inode.entries(&self.block_device).iter().find_map(|entry| {
            if entry.name() == name {
                Some(entry.inode_id())
            } else {
                None
            }
        })
    }
    /// Find inode under current inode by name
    pub fn find(&self, name: &str) -> Option<Arc<Inode>> {
        let fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| {
            self.find_inode_id(name, disk_inode).map(|inode_id| {
                let (block_id, block_offset) = fs.get_disk_inode_pos(inode_id);
                Arc::new(Self::new(
                    block_id,
                    block_offset,
                    self.fs.clone(),
                    self.block_device.clone(),
                ))
            })
        })
    }
    /// Increase the size of a disk inode
    fn increase_size(
        &self,
        new_size: u32,
        disk_inode: &mut DiskInode,
        fs: &mut MutexGuard<EasyFileSystem>,
    ) {
        if new_size < disk_inode.size {
            return;
        }
        let blocks_needed = disk_inode.blocks_num_needed(new_size);
        let mut v: Vec<u32> = Vec::new();
        for _ in 0..blocks_needed {
            v.push(fs.alloc_data());
        }
        disk_inode.increase_size(new_size, v, &self.block_device);
    }


    /// Create inode under current inode by name
    pub fn create(&self, name: &str) -> Option<Arc<Inode>> {
        let mut fs = self.fs.lock();
        let op = |root_inode: &DiskInode| {
            // assert it is a directory
            assert!(root_inode.is_dir());
            // has the file been created?
            self.find_inode_id(name, root_inode)
        };
        if self.read_disk_inode(op).is_some() {
            return None;
        }
        // create a new file
        // alloc an inode with an indirect block
        let new_inode_id = fs.alloc_inode();
        // initialize inode
        let (new_inode_block_id, new_inode_block_offset) = fs.get_disk_inode_pos(new_inode_id);
        get_block_cache(new_inode_block_id as usize, Arc::clone(&self.block_device))
            .lock()
            .modify(new_inode_block_offset, |new_inode: &mut DiskInode| {
                new_inode.initialize(DiskInodeType::File);
                new_inode.set_inode_id(new_inode_id)
            });
        self.modify_disk_inode(|root_inode| {
            // append file in the dirent
            let file_count = (root_inode.size as usize) / DIRENT_SZ;
            let new_size = (file_count + 1) * DIRENT_SZ;
            // increase size
            self.increase_size(new_size as u32, root_inode, &mut fs);
            // write dirent
            let dirent = DirEntry::new(name, new_inode_id);
            root_inode.write_at(
                file_count * DIRENT_SZ,
                dirent.as_bytes(),
                &self.block_device,
            );
        });

        let (block_id, block_offset) = fs.get_disk_inode_pos(new_inode_id);
        block_cache_sync_all();
        // return inode
        Some(Arc::new(Self::new(
            block_id,
            block_offset,
            self.fs.clone(),
            self.block_device.clone(),
        )))
        // release efs lock automatically by compiler
    }
    /// List inodes under current inode
    pub fn ls(&self) -> Vec<String> {
        let _fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| {
            disk_inode.entries(&self.block_device).iter().map(|entry| entry.name().to_string()).collect()
        })
    }
    /// Read data from current inode
    pub fn read_at(&self, offset: usize, buf: &mut [u8]) -> usize {
        let _fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| disk_inode.read_at(offset, buf, &self.block_device))
    }
    /// Write data to current inode
    pub fn write_at(&self, offset: usize, buf: &[u8]) -> usize {
        let mut fs = self.fs.lock();
        let size = self.modify_disk_inode(|disk_inode| {
            self.increase_size((offset + buf.len()) as u32, disk_inode, &mut fs);
            disk_inode.write_at(offset, buf, &self.block_device)
        });
        block_cache_sync_all();
        size
    }


    /// Clear the data in current inode
    pub fn clear(&self) {
        let mut fs = self.fs.lock();
        self.modify_disk_inode(|disk_inode| {
            let size = disk_inode.size;
            let data_blocks_dealloc = disk_inode.clear_size(&self.block_device);
            assert_eq!(data_blocks_dealloc.len(), DiskInode::total_blocks(size) as usize);
            for data_block in data_blocks_dealloc.into_iter() {
                fs.dealloc_data(data_block);
            }
        });
        block_cache_sync_all();
    }

    /// Append a dirent to current inode
    pub fn append_entry(&self, name: &str, inode_id: u32) {
        let entry = DirEntry::new(name, inode_id);

        let mut fs = self.fs.lock();
        self.modify_disk_inode(
            |dinode| {
                let file_count = dinode.file_count();
                let new_size = (file_count + 1) * DIRENT_SZ;
                // increase size
                self.increase_size(new_size as u32, dinode, &mut fs);
                dinode.write_at(
                    file_count * DIRENT_SZ,
                    entry.as_bytes(),
                    &self.block_device)
            }
        );
        block_cache_sync_all()
    }


    /// Remove a dirent from current inode
    pub fn remove_entry(&self, name: &str) {
        assert!(self.is_dir());
        self.modify_disk_inode(|dinode| {
            if let Some(i) = dinode.entries(&self.block_device).iter().position(
                |en| en.name() == name
            ) {
                if i + 1 < dinode.file_count() {
                    // remove the last entry

                    // move all entries after the removed one to the left
                    (i + 1..dinode.file_count())
                        .for_each(|entry_index|
                            {
                                let mut entry_trunck = DirEntry::empty();
                                dinode.read_at(entry_index * DIRENT_SZ, entry_trunck.as_bytes_mut(), &self.block_device);

                                dinode.write_at((entry_index - 1) * DIRENT_SZ, entry_trunck.as_bytes(), &self.block_device);
                            })
                }
            }
            dinode.size -= DIRENT_SZ as u32;
        });
        block_cache_sync_all();
    }
}
