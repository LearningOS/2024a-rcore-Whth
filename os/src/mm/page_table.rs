//! Implementation of [`PageTableEntry`] and [`PageTable`].

use super::{frame_alloc, FrameTracker, PhysPageNum, StepByOne, VirtAddr, VirtPageNum};
use crate::task::current_user_token;
use alloc::vec;
use alloc::vec::Vec;
use bitflags::*;
use core::mem::size_of;
use core::slice;

bitflags! {
    /// page table entry flags
    pub struct PTEFlags: u8 {
        const V = 1 << 0;
        const R = 1 << 1;
        const W = 1 << 2;
        const X = 1 << 3;
        const U = 1 << 4;
        const G = 1 << 5;
        const A = 1 << 6;
        const D = 1 << 7;
    }
}

#[derive(Copy, Clone)]
#[repr(C)]
/// page table entry structure
pub struct PageTableEntry {
    /// bits of page table entry
    pub bits: usize,
}

impl PageTableEntry {
    /// Create a new page table entry
    pub fn new(ppn: PhysPageNum, flags: PTEFlags) -> Self {
        PageTableEntry {
            bits: ppn.0 << 10 | flags.bits as usize,
        }
    }
    /// Create an empty page table entry
    pub fn empty() -> Self {
        PageTableEntry { bits: 0 }
    }
    /// Get the physical page number from the page table entry
    pub fn ppn(&self) -> PhysPageNum {
        (self.bits >> 10 & ((1usize << 44) - 1)).into()
    }
    /// Get the flags from the page table entry
    pub fn flags(&self) -> PTEFlags {
        PTEFlags::from_bits(self.bits as u8).unwrap()
    }
    /// The page pointered by page table entry is valid?
    pub fn is_valid(&self) -> bool {
        (self.flags() & PTEFlags::V) != PTEFlags::empty()
    }
    /// The page pointered by page table entry is readable?
    pub fn readable(&self) -> bool {
        (self.flags() & PTEFlags::R) != PTEFlags::empty()
    }
    /// The page pointered by page table entry is writable?
    pub fn writable(&self) -> bool {
        (self.flags() & PTEFlags::W) != PTEFlags::empty()
    }
    /// The page pointered by page table entry is executable?
    pub fn executable(&self) -> bool {
        (self.flags() & PTEFlags::X) != PTEFlags::empty()
    }
}

/// page table structure
pub struct PageTable {
    root_ppn: PhysPageNum,
    frames: Vec<FrameTracker>,
}

/// Assume that it won't oom when creating/mapping.
impl PageTable {
    /// Create a new page table
    pub fn new() -> Self {
        let frame = frame_alloc().unwrap();
        PageTable {
            root_ppn: frame.ppn,
            frames: vec![frame],
        }
    }
    /// Temporarily used to get arguments from user space.
    pub fn from_token(satp: usize) -> Self {
        Self {
            root_ppn: PhysPageNum::from(satp & ((1usize << 44) - 1)),
            frames: Vec::new(),
        }
    }
    /// Find PageTableEntry by VirtPageNum, create a frame for a 4KB page table if not exist
    fn find_pte_create(&mut self, vpn: VirtPageNum) -> Option<&mut PageTableEntry> {
        let idxs = vpn.indexes();
        let mut ppn = self.root_ppn;
        let mut result: Option<&mut PageTableEntry> = None;
        for (i, idx) in idxs.iter().enumerate() {
            let pte = &mut ppn.get_pte_array()[*idx];
            if i == 2 {
                result = Some(pte);
                break;
            }
            if !pte.is_valid() {
                let frame = frame_alloc().unwrap();
                *pte = PageTableEntry::new(frame.ppn, PTEFlags::V);
                self.frames.push(frame);
            }
            ppn = pte.ppn();
        }
        result
    }
    /// Find PageTableEntry by VirtPageNum
    fn find_pte(&self, vpn: VirtPageNum) -> Option<&mut PageTableEntry> {
        let idxs = vpn.indexes();
        let mut ppn = self.root_ppn;
        let mut result: Option<&mut PageTableEntry> = None;
        for (i, idx) in idxs.iter().enumerate() {
            let pte = &mut ppn.get_pte_array()[*idx];
            if i == 2 {
                result = Some(pte);
                break;
            }
            if !pte.is_valid() {
                return None;
            }
            ppn = pte.ppn();
        }
        result
    }
    /// set the map between virtual page number and physical page number
    #[allow(unused)]
    pub fn map(&mut self, vpn: VirtPageNum, ppn: PhysPageNum, flags: PTEFlags) {
        let pte = self.find_pte_create(vpn).unwrap();
        assert!(!pte.is_valid(), "vpn {:?} is mapped before mapping", vpn);
        *pte = PageTableEntry::new(ppn, flags | PTEFlags::V);
    }
    /// remove the map between virtual page number and physical page number
    #[allow(unused)]
    pub fn unmap(&mut self, vpn: VirtPageNum) {
        let pte = self.find_pte(vpn).unwrap();
        assert!(pte.is_valid(), "vpn {:?} is invalid before unmapping", vpn);
        *pte = PageTableEntry::empty();
    }
    /// get the page table entry from the virtual page number
    pub fn translate(&self, vpn: VirtPageNum) -> Option<PageTableEntry> {
        self.find_pte(vpn).map(|pte| *pte)
    }
    /// get the token from the page table
    pub fn token(&self) -> usize {
        8usize << 60 | self.root_ppn.0
    }
}

/// Translate&Copy a ptr[u8] array with LENGTH len to a mutable u8 Vec through page table
///
/// # Parameters
/// - `token`: The root token for the page table
/// - `ptr`: The pointer to the start of the u8 array
/// - `len`: The length of the u8 array
///
/// # Returns
/// - Returns a Vec containing mutable references to u8
pub fn translated_byte_buffer(token: usize, ptr: *const u8, len: usize) -> Vec<&'static mut u8> {
    // Obtain the page table from the root token
    let page_table = PageTable::from_token(token);
    // Convert the pointer to a usize value for calculation
    let mut start = ptr as usize;
    let end = start + len;
    let mut v = Vec::new();
    while start < end {
        // Convert the start address to a virtual address
        let start_va = VirtAddr::from(start);
        // Calculate the virtual page number (VPN) of the start address
        let mut vpn = start_va.floor();
        // Translate the VPN to a physical page number (PPN)
        let ppn = page_table.translate(vpn).unwrap().ppn();
        // Move the VPN to the next page
        vpn.step();
        // Convert the next VPN to a virtual address, ensuring it does not exceed the end address
        let mut end_va: VirtAddr = vpn.into();
        end_va = end_va.min(VirtAddr::from(end));
        // Determine if the end address of the current page is the start of a new page
        if end_va.page_offset() == 0 {
            // If the end address is the start of a new page, add all the bytes of the current page to the Vec
            v.extend(&mut ppn.get_bytes_array()[start_va.page_offset()..]);
        } else {
            // If the end address is not the start of a new page, add the bytes of the current page to the Vec according to the end offset
            v.extend(&mut ppn.get_bytes_array()[start_va.page_offset()..end_va.page_offset()]);
        }
        // Update the start address to the end address of the current page
        start = end_va.into();
    }
    v
}


/// 从当前用户空间复制数据到目标缓冲区
///
/// # Parameters
///
/// - `dst`: 目标缓冲区，数据将被复制到此处
/// - `src`: 源地址，从当前用户空间复制数据的起始位置
///
/// # Returns
///
/// - `isize`: 复制操作的结果，0表示成功
///
/// # Safety
///
/// 此函数涉及直接内存操作和权限提升，可能访问非法内存或引起安全问题
#[allow(dead_code)]
pub fn copy_from_cur_user<T>(dst: &mut T, src: *const T) -> isize
where
    T: Sized,
{
    // 获取当前用户的令牌，用于后续的数据翻译
    let token: usize = current_user_token();

    // 将目标缓冲区转换为u8类型的指针，以便进行字节级操作
    let dst_ptr = dst as *mut T as *mut u8;
    // 获取类型T的大小，用于确定要复制的字节数
    let t_size = size_of::<T>();
    // 创建一个可变切片，用于表示目标数据块
    let dst_segments = unsafe { slice::from_raw_parts_mut(dst_ptr, t_size) };
    // 根据当前用户令牌，将源地址翻译成一个数据块
    let src_segments = translated_byte_buffer(token, src as *const u8, t_size);

    // 遍历每个字节，将源数据块中的内容复制到目标数据块中
    for ind in 0..t_size {
        // 将源数据块中的每个字节复制到目标数据块中
        dst_segments[ind] = *src_segments[ind];
    }
    // 返回0，表示复制操作成功完成
    0
}


/// 将数据从当前用户空间的源地址复制到目标地址
/// 此函数主要用于在用户空间内进行数据传输，特别是在不同地址之间复制数据时
/// 它通过直接操作内存来实现数据的复制，确保了数据的完整性和效率
///
/// # 参数
///
/// * `dst` - 一个指向目标位置的可变指针，表示数据将被复制到哪里
/// * `src` - 一个指向源数据的引用，表示将从哪里复制数据
///
/// # 返回值
///
/// * `isize` - 复制操作成功时返回0，否则返回非零值表示错误
///
/// # 安全性
///
/// 此函数涉及直接内存操作，因此需要确保提供的指针有效且指向的内存区域可访问
/// 否则，可能会导致程序崩溃或未定义行为
pub fn copy_to_cur_user<T>(dst: *mut T, src: &T) -> isize
where
    T: Sized,
{

    // 将源数据转换为u8类型的指针，以便逐字节操作
    let src_ptr: *const u8 = src as *const T as *const u8;

    let t_size = size_of::<T>();
    // 获取当前用户的token，用于后续的内存翻译
    let token = current_user_token();
    // 将目标地址翻译为当前用户空间中的字节缓冲区
    let mut dst_segments = translated_byte_buffer(token, dst as *const u8, t_size);


    // 遍历目标字节缓冲区，逐块复制源数据到目标位置

    let src_segments = unsafe { slice::from_raw_parts(src_ptr, t_size) };

    for ind in 0..t_size {
        *(dst_segments[ind]) = src_segments[ind];
    }
    // 复制操作成功完成，返回0表示无错误
    0
}
