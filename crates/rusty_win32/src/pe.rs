//! `pe` — a zero-copy, allocation-free parser for the on-disk layout of a
//! Portable Executable image (`.exe`/`.dll`/`.sys`), the read half of a PE
//! loader.
//!
//! Every other module in this crate is a thin safe wrapper around a
//! documented `kernel32`/`advapi32`/`ws2_32`/… export reached via
//! `unsafe extern "system"` FFI (see `ARCHITECTURE.md`). This module is the
//! deliberate exception: the Portable Executable *format* is Windows' own,
//! but reading it is pure byte-parsing with no OS call to wrap and, for the
//! same reason, no `unsafe` anywhere in it — an auditability property none of
//! the FFI modules can claim. Like [`crate::error`], it therefore stays
//! available off-Windows (its input is a `&[u8]`, not a live handle), so a
//! caller can inspect a Windows binary from any host — the parser has no
//! `#[cfg(windows)]` gate.
//!
//! What it answers is the "what does this image *declare*" half of loading —
//! the metadata Windows' own loader reads before mapping: the target
//! [`Machine`], whether the image is a DLL or an executable, its
//! [`Subsystem`] (console vs GUI — which a shell needs to decide how to
//! launch and wait on a child), its entry point and preferred image base,
//! its [`Section`]s, its [`data_directory`](PeFile::data_directory) table,
//! the symbols it [`exports`](PeFile::exports) (the on-disk complement of
//! [`crate::dynlib::get_proc_address`], which answers the same question for a
//! module the OS has *already* mapped), and the modules it
//! [`imports`](PeFile::imports).
//!
//! What it deliberately does **not** do is the *other* half of loading —
//! allocating image memory, mapping sections to their virtual addresses,
//! applying base relocations, resolving the import address table, running
//! TLS callbacks, and calling the entry point. That is runtime logic, not an
//! FFI wrapper or a byte parse, and hosting a foreign image in-process is
//! exactly the manual-mapping technique this crate's thin-wrapper,
//! no-runtime-logic charter has no place for — a caller that actually wants
//! an image *loaded and run* should hand its path to `CreateProcessW`
//! ([`crate::process`]) or `LoadLibraryW` ([`crate::dynlib`]) and let the OS
//! loader do it.
//!
//! All offsets are into the file's on-disk bytes. A separate in-memory view
//! (parsing an already-mapped module from its `HMODULE` base, where sections
//! sit at their virtual addresses rather than their file offsets) would be a
//! natural Windows-only follow-up but is intentionally not conflated with
//! the on-disk parse here.

/// A structural failure parsing a PE image.
///
/// These are *format* errors, not `GetLastError()` codes, so — unlike every
/// other fallible operation in this crate — they do not funnel through
/// [`crate::error::Win32Error`]: no Win32 call is made, so there is no last-
/// error to read, and a raw byte parse's failure modes (a truncated buffer,
/// a bad signature) map onto precise, matchable variants far better than onto
/// `ERROR_BAD_EXE_FORMAT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeError {
    /// The buffer is smaller than a header the format requires to be present.
    TooSmall,
    /// The `IMAGE_DOS_HEADER` does not begin with the `MZ` magic.
    BadDosSignature,
    /// The `e_lfanew` offset does not point at the `PE\0\0` signature.
    BadPeSignature,
    /// The optional header's `Magic` is neither `PE32` (`0x10b`) nor `PE32+`
    /// (`0x20b`); the carried value is the magic that was found.
    BadOptionalMagic(u16),
    /// A header field referenced bytes past the end of the buffer.
    OutOfBounds,
}

impl core::fmt::Display for PeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            PeError::TooSmall => f.write_str("PE image is smaller than a required header"),
            PeError::BadDosSignature => f.write_str("PE image has no MZ DOS signature"),
            PeError::BadPeSignature => {
                f.write_str("PE image has no PE\\0\\0 signature at e_lfanew")
            }
            PeError::BadOptionalMagic(m) => {
                write!(f, "PE optional header has an unrecognized magic {m:#06x}")
            }
            PeError::OutOfBounds => {
                f.write_str("a PE header field referenced data past the end of the image")
            }
        }
    }
}

impl core::error::Error for PeError {}

/// The target machine an image is built for — `IMAGE_FILE_HEADER.Machine`.
///
/// A newtype over the raw `u16` rather than a closed enum: the set of machine
/// values grows as Microsoft adds architectures, and preserving an
/// unrecognized value (`.raw()` still works, `is_known()` returns `false`) is
/// truer to the on-disk byte than silently mapping it to an "unknown" variant
/// would be. The named constants cover the machines a modern Windows actually
/// runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Machine(pub u16);

impl Machine {
    /// `IMAGE_FILE_MACHINE_UNKNOWN` — applicable to any machine.
    pub const UNKNOWN: Machine = Machine(0x0000);
    /// `IMAGE_FILE_MACHINE_I386` — x86 (32-bit).
    pub const I386: Machine = Machine(0x014c);
    /// `IMAGE_FILE_MACHINE_AMD64` — x64.
    pub const AMD64: Machine = Machine(0x8664);
    /// `IMAGE_FILE_MACHINE_ARMNT` — ARM Thumb-2 (32-bit).
    pub const ARMNT: Machine = Machine(0x01c4);
    /// `IMAGE_FILE_MACHINE_ARM64` — ARM64.
    pub const ARM64: Machine = Machine(0xaa64);
    /// `IMAGE_FILE_MACHINE_IA64` — Intel Itanium.
    pub const IA64: Machine = Machine(0x0200);

    /// The raw `u16` machine value, exactly as it appears in the header.
    pub const fn raw(self) -> u16 {
        self.0
    }

    /// Whether this value matches one of the named machine constants.
    pub const fn is_known(self) -> bool {
        matches!(
            self,
            Machine::UNKNOWN
                | Machine::I386
                | Machine::AMD64
                | Machine::ARMNT
                | Machine::ARM64
                | Machine::IA64
        )
    }
}

/// The Windows subsystem an image targets — `IMAGE_OPTIONAL_HEADER.Subsystem`.
///
/// A newtype over the raw `u16` for the same reason as [`Machine`]. The two
/// values a shell most cares about — [`WINDOWS_GUI`](Subsystem::WINDOWS_GUI)
/// and [`WINDOWS_CUI`](Subsystem::WINDOWS_CUI) — decide whether launching the
/// image detaches from or shares the console.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Subsystem(pub u16);

impl Subsystem {
    /// `IMAGE_SUBSYSTEM_UNKNOWN`.
    pub const UNKNOWN: Subsystem = Subsystem(0);
    /// `IMAGE_SUBSYSTEM_NATIVE` — a driver / native image (no subsystem).
    pub const NATIVE: Subsystem = Subsystem(1);
    /// `IMAGE_SUBSYSTEM_WINDOWS_GUI` — a windowed application.
    pub const WINDOWS_GUI: Subsystem = Subsystem(2);
    /// `IMAGE_SUBSYSTEM_WINDOWS_CUI` — a console application.
    pub const WINDOWS_CUI: Subsystem = Subsystem(3);
    /// `IMAGE_SUBSYSTEM_EFI_APPLICATION`.
    pub const EFI_APPLICATION: Subsystem = Subsystem(10);

    /// The raw `u16` subsystem value, exactly as it appears in the header.
    pub const fn raw(self) -> u16 {
        self.0
    }

    /// Whether this is the console (`WINDOWS_CUI`) subsystem.
    pub const fn is_console(self) -> bool {
        self.0 == Subsystem::WINDOWS_CUI.0
    }

    /// Whether this is the GUI (`WINDOWS_GUI`) subsystem.
    pub const fn is_gui(self) -> bool {
        self.0 == Subsystem::WINDOWS_GUI.0
    }
}

// --- `IMAGE_FILE_HEADER.Characteristics` bits (the two a caller usually
// asks about are surfaced as predicates on `PeFile`; the raw field is also
// exposed via `characteristics()`). ---

/// `IMAGE_FILE_EXECUTABLE_IMAGE` — the image is a runnable executable (as
/// opposed to an object file); set on any normal `.exe`/`.dll`.
pub const IMAGE_FILE_EXECUTABLE_IMAGE: u16 = 0x0002;
/// `IMAGE_FILE_DLL` — the image is a dynamic-link library, not a standalone
/// program.
pub const IMAGE_FILE_DLL: u16 = 0x2000;
/// `IMAGE_FILE_LARGE_ADDRESS_AWARE` — the app can handle addresses above 2 GB.
pub const IMAGE_FILE_LARGE_ADDRESS_AWARE: u16 = 0x0020;
/// `IMAGE_FILE_32BIT_MACHINE` — the image is for a 32-bit machine word.
pub const IMAGE_FILE_32BIT_MACHINE: u16 = 0x0100;

// --- Optional-header magics ---
const PE32_MAGIC: u16 = 0x010b;
const PE32PLUS_MAGIC: u16 = 0x020b;

// --- Data-directory indices (`IMAGE_DIRECTORY_ENTRY_*`). ---

/// The export table's data-directory index (`IMAGE_DIRECTORY_ENTRY_EXPORT`).
pub const DIRECTORY_ENTRY_EXPORT: usize = 0;
/// The import table's data-directory index (`IMAGE_DIRECTORY_ENTRY_IMPORT`).
pub const DIRECTORY_ENTRY_IMPORT: usize = 1;
/// The resource table's data-directory index
/// (`IMAGE_DIRECTORY_ENTRY_RESOURCE`).
pub const DIRECTORY_ENTRY_RESOURCE: usize = 2;
/// The base-relocation table's data-directory index
/// (`IMAGE_DIRECTORY_ENTRY_BASERELOC`).
pub const DIRECTORY_ENTRY_BASERELOC: usize = 5;
/// The TLS table's data-directory index (`IMAGE_DIRECTORY_ENTRY_TLS`).
pub const DIRECTORY_ENTRY_TLS: usize = 9;

const SECTION_HEADER_SIZE: usize = 40;
const DATA_DIRECTORY_SIZE: usize = 8;

// --- Bounds-checked little-endian scalar reads. Every field access in this
// module goes through one of these, so an out-of-bounds header can only ever
// produce `PeError::OutOfBounds`, never a panic. ---

fn read_u16(bytes: &[u8], off: usize) -> Result<u16, PeError> {
    bytes
        .get(off..off + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or(PeError::OutOfBounds)
}

fn read_u32(bytes: &[u8], off: usize) -> Result<u32, PeError> {
    bytes
        .get(off..off + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or(PeError::OutOfBounds)
}

fn read_u64(bytes: &[u8], off: usize) -> Result<u64, PeError> {
    bytes
        .get(off..off + 8)
        .map(|s| u64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]))
        .ok_or(PeError::OutOfBounds)
}

/// A NUL-terminated ASCII/UTF-8 string embedded at `off` in the image, as a
/// borrowed `&str`. Bytes up to (not including) the first NUL are returned;
/// invalid UTF-8 or a missing NUL before end-of-buffer yields `None`. PE name
/// strings (export names, imported DLL names) are ASCII in practice, so this
/// is lossless for them.
fn read_cstr(bytes: &[u8], off: usize) -> Option<&str> {
    let rest = bytes.get(off..)?;
    let end = rest.iter().position(|&b| b == 0)?;
    core::str::from_utf8(&rest[..end]).ok()
}

/// A parsed, borrowed view of a PE image's on-disk headers.
///
/// Holds the original `&[u8]` plus the handful of offsets and fields the
/// header walk resolved once, so accessors and the [`sections`](Self::sections)
/// / [`exports`](Self::exports) / [`imports`](Self::imports) iterators are
/// cheap. Construct with [`parse`](Self::parse).
#[derive(Debug, Clone, Copy)]
pub struct PeFile<'a> {
    bytes: &'a [u8],
    machine: Machine,
    characteristics: u16,
    is_64bit: bool,
    subsystem: Subsystem,
    dll_characteristics: u16,
    entry_point: u32,
    image_base: u64,
    size_of_image: u32,
    section_alignment: u32,
    file_alignment: u32,
    number_of_sections: u16,
    sections_offset: usize,
    number_of_rva_and_sizes: u32,
    data_directory_offset: usize,
}

impl<'a> PeFile<'a> {
    /// Parse the headers of a PE image from its on-disk bytes.
    ///
    /// Validates the `MZ` and `PE\0\0` signatures and the optional-header
    /// magic, then reads the COFF and optional headers. Section, export, and
    /// import contents are parsed lazily by the respective iterators, not
    /// here — so a successful parse guarantees the *headers* are well-formed
    /// and in-bounds, but a later iterator can still surface
    /// [`PeError::OutOfBounds`] for a directory that points outside the file.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, PeError> {
        // IMAGE_DOS_HEADER: `MZ` magic at 0, `e_lfanew` (offset of the NT
        // headers) at 0x3C.
        if bytes.len() < 64 {
            return Err(PeError::TooSmall);
        }
        if read_u16(bytes, 0)? != 0x5a4d {
            return Err(PeError::BadDosSignature);
        }
        let pe_offset = read_u32(bytes, 0x3c)? as usize;

        // IMAGE_NT_HEADERS: 4-byte `PE\0\0` signature, then the 20-byte
        // IMAGE_FILE_HEADER (COFF), then the optional header.
        if read_u32(bytes, pe_offset)? != 0x0000_4550 {
            return Err(PeError::BadPeSignature);
        }
        let coff = pe_offset + 4;
        let machine = Machine(read_u16(bytes, coff)?);
        let number_of_sections = read_u16(bytes, coff + 2)?;
        let size_of_optional_header = read_u16(bytes, coff + 16)? as usize;
        let characteristics = read_u16(bytes, coff + 18)?;

        // IMAGE_OPTIONAL_HEADER — its `Magic` selects the PE32 vs PE32+
        // layout. Only `ImageBase`, `NumberOfRvaAndSizes`, and the data-
        // directory array's start differ between the two; every other field
        // this parser reads sits at the same offset in both.
        let opt = coff + 20;
        let magic = read_u16(bytes, opt)?;
        let is_64bit = match magic {
            PE32_MAGIC => false,
            PE32PLUS_MAGIC => true,
            other => return Err(PeError::BadOptionalMagic(other)),
        };

        let entry_point = read_u32(bytes, opt + 16)?;
        let section_alignment = read_u32(bytes, opt + 32)?;
        let file_alignment = read_u32(bytes, opt + 36)?;
        let size_of_image = read_u32(bytes, opt + 56)?;
        let subsystem = Subsystem(read_u16(bytes, opt + 68)?);
        let dll_characteristics = read_u16(bytes, opt + 70)?;

        let (image_base, number_of_rva_and_sizes, data_directory_offset) = if is_64bit {
            (
                read_u64(bytes, opt + 24)?,
                read_u32(bytes, opt + 108)?,
                opt + 112,
            )
        } else {
            (
                read_u32(bytes, opt + 28)? as u64,
                read_u32(bytes, opt + 92)?,
                opt + 96,
            )
        };

        // Section headers begin immediately after the optional header, whose
        // length the COFF header reports (it is not a fixed size — the data-
        // directory count varies).
        let sections_offset = opt + size_of_optional_header;

        Ok(PeFile {
            bytes,
            machine,
            characteristics,
            is_64bit,
            subsystem,
            dll_characteristics,
            entry_point,
            image_base,
            size_of_image,
            section_alignment,
            file_alignment,
            number_of_sections,
            sections_offset,
            number_of_rva_and_sizes,
            data_directory_offset,
        })
    }

    /// The target machine — `IMAGE_FILE_HEADER.Machine`.
    pub const fn machine(self) -> Machine {
        self.machine
    }

    /// Whether the optional header is `PE32+` (a 64-bit image). `false` for a
    /// `PE32` (32-bit) image.
    pub const fn is_64bit(self) -> bool {
        self.is_64bit
    }

    /// The raw `IMAGE_FILE_HEADER.Characteristics` bitmask (test it against
    /// the `IMAGE_FILE_*` constants).
    pub const fn characteristics(self) -> u16 {
        self.characteristics
    }

    /// Whether the image is a DLL (`IMAGE_FILE_DLL`) rather than a standalone
    /// executable.
    pub const fn is_dll(self) -> bool {
        self.characteristics & IMAGE_FILE_DLL != 0
    }

    /// Whether the image is a runnable executable image
    /// (`IMAGE_FILE_EXECUTABLE_IMAGE`) — set on both `.exe`s and `.dll`s.
    pub const fn is_executable(self) -> bool {
        self.characteristics & IMAGE_FILE_EXECUTABLE_IMAGE != 0
    }

    /// The target subsystem — `IMAGE_OPTIONAL_HEADER.Subsystem` (console vs
    /// GUI, etc.).
    pub const fn subsystem(self) -> Subsystem {
        self.subsystem
    }

    /// The raw `IMAGE_OPTIONAL_HEADER.DllCharacteristics` bitmask (ASLR/DEP/CFG
    /// and similar flags).
    pub const fn dll_characteristics(self) -> u16 {
        self.dll_characteristics
    }

    /// The entry point as an RVA — `IMAGE_OPTIONAL_HEADER.AddressOfEntryPoint`
    /// (offset from [`image_base`](Self::image_base) once loaded; `0` for a
    /// resource-only DLL).
    pub const fn entry_point(self) -> u32 {
        self.entry_point
    }

    /// The preferred load address — `IMAGE_OPTIONAL_HEADER.ImageBase`. Widened
    /// to `u64` for a uniform type across PE32 and PE32+.
    pub const fn image_base(self) -> u64 {
        self.image_base
    }

    /// The size of the image once mapped —
    /// `IMAGE_OPTIONAL_HEADER.SizeOfImage`.
    pub const fn size_of_image(self) -> u32 {
        self.size_of_image
    }

    /// The section alignment in memory —
    /// `IMAGE_OPTIONAL_HEADER.SectionAlignment`.
    pub const fn section_alignment(self) -> u32 {
        self.section_alignment
    }

    /// The alignment of raw section data on disk —
    /// `IMAGE_OPTIONAL_HEADER.FileAlignment`.
    pub const fn file_alignment(self) -> u32 {
        self.file_alignment
    }

    /// The number of sections — `IMAGE_FILE_HEADER.NumberOfSections`.
    pub const fn section_count(self) -> u16 {
        self.number_of_sections
    }

    /// An iterator over the image's section headers, in file order.
    pub fn sections(self) -> Sections<'a> {
        Sections {
            bytes: self.bytes,
            offset: self.sections_offset,
            remaining: self.number_of_sections,
        }
    }

    /// The `index`th data directory (`IMAGE_DATA_DIRECTORY`), or `None` if the
    /// image declares fewer than `index + 1` directories
    /// (`NumberOfRvaAndSizes`). A present-but-empty directory returns a
    /// [`DataDirectory`] with a zero [`size`](DataDirectory::size). Use the
    /// `DIRECTORY_ENTRY_*` constants for `index`.
    pub fn data_directory(self, index: usize) -> Option<DataDirectory> {
        if index >= self.number_of_rva_and_sizes as usize {
            return None;
        }
        let off = self.data_directory_offset + index * DATA_DIRECTORY_SIZE;
        let virtual_address = read_u32(self.bytes, off).ok()?;
        let size = read_u32(self.bytes, off + 4).ok()?;
        Some(DataDirectory {
            virtual_address,
            size,
        })
    }

    /// Translate a relative virtual address to an offset into the on-disk
    /// bytes, by finding the section that contains it and applying that
    /// section's file-vs-virtual delta. Returns `None` if no section covers
    /// `rva` (e.g. it falls in the headers or in uninitialized `.bss`-style
    /// space with no raw data), or if the resulting offset lies outside the
    /// buffer.
    pub fn rva_to_offset(self, rva: u32) -> Option<usize> {
        for section in self.sections() {
            let start = section.virtual_address;
            // A section occupies the larger of its virtual and raw sizes in
            // RVA space; use virtual size, falling back to raw when the
            // header leaves virtual size zero (common for object-style
            // images).
            let span = if section.virtual_size != 0 {
                section.virtual_size
            } else {
                section.size_of_raw_data
            };
            let end = start.checked_add(span)?;
            if rva >= start && rva < end {
                let delta = rva - start;
                if delta >= section.size_of_raw_data {
                    // Inside the section's virtual span but past its raw data
                    // — no on-disk bytes back this RVA.
                    return None;
                }
                let offset = (section.pointer_to_raw_data as usize).checked_add(delta as usize)?;
                return if offset < self.bytes.len() {
                    Some(offset)
                } else {
                    None
                };
            }
        }
        None
    }

    /// The image's export directory, or `None` if it declares no exports
    /// (no [`DIRECTORY_ENTRY_EXPORT`] directory, an empty one, or one whose
    /// RVA does not resolve to on-disk bytes). Iterate the returned
    /// [`Exports`] for the exported names.
    pub fn exports(self) -> Option<Exports<'a>> {
        let dir = self.data_directory(DIRECTORY_ENTRY_EXPORT)?;
        if dir.size == 0 || dir.virtual_address == 0 {
            return None;
        }
        let table = self.rva_to_offset(dir.virtual_address)?;
        // IMAGE_EXPORT_DIRECTORY: Base@16, NumberOfNames@24,
        // AddressOfNames@32, AddressOfNameOrdinals@36.
        let ordinal_base = read_u32(self.bytes, table + 16).ok()?;
        let number_of_names = read_u32(self.bytes, table + 24).ok()?;
        let address_of_names = read_u32(self.bytes, table + 32).ok()?;
        let address_of_name_ordinals = read_u32(self.bytes, table + 36).ok()?;
        let names_offset = self.rva_to_offset(address_of_names)?;
        let ordinals_offset = self.rva_to_offset(address_of_name_ordinals)?;
        Some(Exports {
            file: self,
            names_offset,
            ordinals_offset,
            ordinal_base,
            count: number_of_names,
            index: 0,
        })
    }

    /// The image's imported modules, or `None` if it declares no imports
    /// (no [`DIRECTORY_ENTRY_IMPORT`] directory, an empty one, or one whose
    /// RVA does not resolve to on-disk bytes). Iterate the returned
    /// [`Imports`] for each dependency's DLL name.
    pub fn imports(self) -> Option<Imports<'a>> {
        let dir = self.data_directory(DIRECTORY_ENTRY_IMPORT)?;
        if dir.size == 0 || dir.virtual_address == 0 {
            return None;
        }
        let descriptors = self.rva_to_offset(dir.virtual_address)?;
        Some(Imports {
            file: self,
            offset: descriptors,
            done: false,
        })
    }
}

/// One entry from the data-directory table (`IMAGE_DATA_DIRECTORY`): the RVA
/// and byte size of a well-known table (exports, imports, relocations, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataDirectory {
    /// The table's relative virtual address (`0` if absent).
    pub virtual_address: u32,
    /// The table's size in bytes (`0` if absent).
    pub size: u32,
}

/// A single section header (`IMAGE_SECTION_HEADER`), yielded by
/// [`PeFile::sections`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Section<'a> {
    /// The raw 8-byte `Name` field, unmodified (NUL-padded on the right).
    /// Use [`name`](Self::name) for the trimmed `&str`.
    pub raw_name: &'a [u8],
    /// `VirtualSize` — the section's size once mapped into memory.
    pub virtual_size: u32,
    /// `VirtualAddress` — the section's RVA once mapped.
    pub virtual_address: u32,
    /// `SizeOfRawData` — the section's size on disk.
    pub size_of_raw_data: u32,
    /// `PointerToRawData` — the section's file offset.
    pub pointer_to_raw_data: u32,
    /// `Characteristics` — the section's flags (`IMAGE_SCN_*`: executable,
    /// readable, writable, …).
    pub characteristics: u32,
}

impl Section<'_> {
    /// The section name as a `&str`, trimmed of the `Name` field's trailing
    /// NUL padding. Returns `None` for the rare long name encoded as a
    /// `/<offset>` reference into the COFF string table (not resolved here),
    /// or for non-UTF-8 bytes.
    pub fn name(&self) -> Option<&str> {
        let end = self
            .raw_name
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(self.raw_name.len());
        let trimmed = &self.raw_name[..end];
        if trimmed.first() == Some(&b'/') {
            return None;
        }
        core::str::from_utf8(trimmed).ok()
    }
}

/// Iterator over a PE image's section headers. Created by
/// [`PeFile::sections`].
#[derive(Debug, Clone)]
pub struct Sections<'a> {
    bytes: &'a [u8],
    offset: usize,
    remaining: u16,
}

impl<'a> Iterator for Sections<'a> {
    type Item = Section<'a>;

    fn next(&mut self) -> Option<Section<'a>> {
        if self.remaining == 0 {
            return None;
        }
        let off = self.offset;
        // Stop cleanly rather than panic if a bogus NumberOfSections runs the
        // walk off the end of the buffer.
        let raw_name = self.bytes.get(off..off + 8)?;
        let virtual_size = read_u32(self.bytes, off + 8).ok()?;
        let virtual_address = read_u32(self.bytes, off + 12).ok()?;
        let size_of_raw_data = read_u32(self.bytes, off + 16).ok()?;
        let pointer_to_raw_data = read_u32(self.bytes, off + 20).ok()?;
        let characteristics = read_u32(self.bytes, off + 36).ok()?;

        self.offset += SECTION_HEADER_SIZE;
        self.remaining -= 1;
        Some(Section {
            raw_name,
            virtual_size,
            virtual_address,
            size_of_raw_data,
            pointer_to_raw_data,
            characteristics,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(self.remaining as usize))
    }
}

/// One exported symbol, yielded by [`Exports`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Export<'a> {
    /// The exported name.
    pub name: &'a str,
    /// The export ordinal (`ordinal base + name-ordinal-table index`), the
    /// value `GetProcAddress` would accept via `MAKEINTRESOURCE`.
    pub ordinal: u32,
}

/// Iterator over a PE image's named exports. Created by [`PeFile::exports`].
///
/// Walks the export directory's name table (`AddressOfNames`) in parallel
/// with its ordinal table (`AddressOfNameOrdinals`). Nameless (ordinal-only)
/// exports are not yielded — only the names a caller would pass to
/// `GetProcAddress` by string. A malformed entry ends iteration rather than
/// panicking.
#[derive(Debug, Clone)]
pub struct Exports<'a> {
    file: PeFile<'a>,
    names_offset: usize,
    ordinals_offset: usize,
    ordinal_base: u32,
    count: u32,
    index: u32,
}

impl<'a> Iterator for Exports<'a> {
    type Item = Export<'a>;

    fn next(&mut self) -> Option<Export<'a>> {
        if self.index >= self.count {
            return None;
        }
        let i = self.index as usize;
        // AddressOfNames[i] -> RVA of the name string.
        let name_rva = read_u32(self.file.bytes, self.names_offset + i * 4).ok()?;
        let name_offset = self.file.rva_to_offset(name_rva)?;
        let name = read_cstr(self.file.bytes, name_offset)?;
        // AddressOfNameOrdinals[i] -> index into AddressOfFunctions; the
        // public ordinal is that index plus the directory's ordinal base.
        let ordinal_index = read_u16(self.file.bytes, self.ordinals_offset + i * 2).ok()?;
        let ordinal = self.ordinal_base + ordinal_index as u32;

        self.index += 1;
        Some(Export { name, ordinal })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = (self.count - self.index) as usize;
        (0, Some(remaining))
    }
}

/// One imported module, yielded by [`Imports`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Import<'a> {
    /// The imported DLL's name (e.g. `"KERNEL32.dll"`).
    pub name: &'a str,
}

/// Iterator over the modules a PE image imports. Created by
/// [`PeFile::imports`].
///
/// Walks the `IMAGE_IMPORT_DESCRIPTOR` array to its zero terminator, yielding
/// each dependency's DLL name — the "what does this need present to run"
/// question. Per-function import enumeration (walking each descriptor's
/// thunk arrays) is intentionally left out: module-level dependency
/// inspection is the common need, and a 32-vs-64-bit, ordinal-flagged thunk
/// walk is materially more surface than this module aims to carry. A
/// malformed descriptor ends iteration rather than panicking.
#[derive(Debug, Clone)]
pub struct Imports<'a> {
    file: PeFile<'a>,
    offset: usize,
    done: bool,
}

impl<'a> Iterator for Imports<'a> {
    type Item = Import<'a>;

    fn next(&mut self) -> Option<Import<'a>> {
        if self.done {
            return None;
        }
        // IMAGE_IMPORT_DESCRIPTOR is 20 bytes; Name (an RVA to the DLL name)
        // is at +12. The array ends at an all-zero descriptor, detected here
        // by a zero Name field.
        let name_rva = read_u32(self.file.bytes, self.offset + 12).ok()?;
        if name_rva == 0 {
            self.done = true;
            return None;
        }
        let name_offset = self.file.rva_to_offset(name_rva)?;
        let name = read_cstr(self.file.bytes, name_offset)?;
        self.offset += 20;
        Some(Import { name })
    }
}

#[cfg(test)]
mod tests {
    extern crate alloc;
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    // A hand-built PE image just complete enough to exercise the parser:
    // DOS header, NT headers, one ".text" section, and — laid inside that
    // section's raw bytes — an export directory advertising one name and an
    // import descriptor naming one dependency. `is_64bit` selects PE32+ vs
    // PE32 so both optional-header layouts are covered.
    struct Builder {
        buf: Vec<u8>,
    }

    impl Builder {
        fn put_u16(&mut self, off: usize, v: u16) {
            self.buf[off..off + 2].copy_from_slice(&v.to_le_bytes());
        }
        fn put_u32(&mut self, off: usize, v: u32) {
            self.buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
        }
        fn put_u64(&mut self, off: usize, v: u64) {
            self.buf[off..off + 8].copy_from_slice(&v.to_le_bytes());
        }
        fn put_bytes(&mut self, off: usize, v: &[u8]) {
            self.buf[off..off + v.len()].copy_from_slice(v);
        }
    }

    // Layout constants shared by builder and assertions.
    const PE_OFF: usize = 0x80; // e_lfanew
    const SECTION_VA: u32 = 0x1000; // .text virtual address
    const SECTION_RAW: usize = 0x400; // .text file offset
    const SECTION_SIZE: usize = 0x400;

    fn build_pe(is_64bit: bool) -> Vec<u8> {
        let opt_size: usize = if is_64bit { 0xF0 } else { 0xE0 };
        let coff = PE_OFF + 4;
        let opt = coff + 20;
        let sections = opt + opt_size;
        let total = SECTION_RAW + SECTION_SIZE;
        let mut b = Builder {
            buf: vec![0u8; total],
        };

        // --- DOS header ---
        b.put_u16(0, 0x5a4d); // "MZ"
        b.put_u32(0x3c, PE_OFF as u32); // e_lfanew

        // --- NT signature + COFF header ---
        b.put_u32(PE_OFF, 0x0000_4550); // "PE\0\0"
        b.put_u16(coff, if is_64bit { 0x8664 } else { 0x014c }); // Machine
        b.put_u16(coff + 2, 1); // NumberOfSections
        b.put_u16(coff + 16, opt_size as u16); // SizeOfOptionalHeader
        b.put_u16(coff + 18, IMAGE_FILE_EXECUTABLE_IMAGE | IMAGE_FILE_DLL); // Characteristics

        // --- Optional header ---
        b.put_u16(opt, if is_64bit { PE32PLUS_MAGIC } else { PE32_MAGIC });
        b.put_u32(opt + 16, 0x1234); // AddressOfEntryPoint
        b.put_u32(opt + 32, 0x1000); // SectionAlignment
        b.put_u32(opt + 36, 0x200); // FileAlignment
        b.put_u32(opt + 56, 0x4000); // SizeOfImage
        b.put_u16(opt + 68, Subsystem::WINDOWS_CUI.raw()); // Subsystem
        b.put_u16(opt + 70, 0x0140); // DllCharacteristics (ASLR|NX, arbitrary)
        if is_64bit {
            b.put_u64(opt + 24, 0x1_4000_0000); // ImageBase
            b.put_u32(opt + 108, 16); // NumberOfRvaAndSizes
        } else {
            b.put_u32(opt + 28, 0x0040_0000); // ImageBase
            b.put_u32(opt + 92, 16); // NumberOfRvaAndSizes
        }
        let data_dirs = if is_64bit { opt + 112 } else { opt + 96 };

        // --- One ".text" section header ---
        b.put_bytes(sections, b".text\0\0\0");
        b.put_u32(sections + 8, SECTION_SIZE as u32); // VirtualSize
        b.put_u32(sections + 12, SECTION_VA); // VirtualAddress
        b.put_u32(sections + 16, SECTION_SIZE as u32); // SizeOfRawData
        b.put_u32(sections + 20, SECTION_RAW as u32); // PointerToRawData
        b.put_u32(sections + 36, 0x6000_0020); // Characteristics (CODE|EXEC|READ)

        // Helper: an RVA inside the section maps to file offset
        // SECTION_RAW + (rva - SECTION_VA).
        let rva_of = |file_off: usize| -> u32 { SECTION_VA + (file_off - SECTION_RAW) as u32 };

        // --- Export directory laid at the start of the section's raw data ---
        let export_dir = SECTION_RAW;
        let names_array = export_dir + 40; // AddressOfNames array
        let ordinals_array = names_array + 4; // AddressOfNameOrdinals array
        let name_string = ordinals_array + 8; // "ExportedFn\0"
        let dll_name = name_string + 16; // this image's own name

        b.put_u32(export_dir + 16, 5); // Base (ordinal base)
        b.put_u32(export_dir + 20, 1); // NumberOfFunctions
        b.put_u32(export_dir + 24, 1); // NumberOfNames
        b.put_u32(export_dir + 28, rva_of(name_string)); // AddressOfFunctions (reuse, unused by parser)
        b.put_u32(export_dir + 32, rva_of(names_array)); // AddressOfNames
        b.put_u32(export_dir + 36, rva_of(ordinals_array)); // AddressOfNameOrdinals
        b.put_u32(export_dir + 12, rva_of(dll_name)); // Name (module's own)
        b.put_u32(names_array, rva_of(name_string)); // AddressOfNames[0]
        b.put_u16(ordinals_array, 0); // AddressOfNameOrdinals[0] -> ordinal 5
        b.put_bytes(name_string, b"ExportedFn\0");
        b.put_bytes(dll_name, b"self.dll\0");

        // --- Import descriptor laid later in the same section ---
        let import_desc = SECTION_RAW + 0x100;
        let import_name = import_desc + 40; // "OTHER.dll\0"
        b.put_u32(import_desc + 12, rva_of(import_name)); // descriptor[0].Name
        b.put_u32(import_desc + 16, rva_of(import_name)); // FirstThunk (nonzero, unused)
        // descriptor[1] is left all-zero: the terminator.
        b.put_bytes(import_name, b"OTHER.dll\0");

        // Point the export/import data directories at those structures.
        b.put_u32(data_dirs + DIRECTORY_ENTRY_EXPORT * 8, rva_of(export_dir));
        b.put_u32(data_dirs + DIRECTORY_ENTRY_EXPORT * 8 + 4, 0x100); // nonzero size
        b.put_u32(data_dirs + DIRECTORY_ENTRY_IMPORT * 8, rva_of(import_desc));
        b.put_u32(data_dirs + DIRECTORY_ENTRY_IMPORT * 8 + 4, 40); // nonzero size

        b.buf
    }

    #[test]
    fn parses_headers_of_a_pe32plus_image() {
        let image = build_pe(true);
        let pe = PeFile::parse(&image).expect("a well-formed PE32+ image should parse");
        assert_eq!(pe.machine(), Machine::AMD64);
        assert!(pe.machine().is_known());
        assert!(pe.is_64bit());
        assert!(pe.is_dll());
        assert!(pe.is_executable());
        assert!(pe.subsystem().is_console());
        assert!(!pe.subsystem().is_gui());
        assert_eq!(pe.entry_point(), 0x1234);
        assert_eq!(pe.image_base(), 0x1_4000_0000);
        assert_eq!(pe.size_of_image(), 0x4000);
        assert_eq!(pe.section_count(), 1);
    }

    #[test]
    fn parses_headers_of_a_pe32_image() {
        let image = build_pe(false);
        let pe = PeFile::parse(&image).expect("a well-formed PE32 image should parse");
        assert_eq!(pe.machine(), Machine::I386);
        assert!(!pe.is_64bit());
        assert_eq!(pe.image_base(), 0x0040_0000);
        assert_eq!(pe.entry_point(), 0x1234);
        assert!(pe.subsystem().is_console());
    }

    #[test]
    fn iterates_the_section_table() {
        let image = build_pe(true);
        let pe = PeFile::parse(&image).unwrap();
        let sections: Vec<_> = pe.sections().collect();
        assert_eq!(sections.len(), 1);
        let text = sections[0];
        assert_eq!(text.name(), Some(".text"));
        assert_eq!(text.virtual_address, SECTION_VA);
        assert_eq!(text.pointer_to_raw_data, SECTION_RAW as u32);
        assert_eq!(text.characteristics, 0x6000_0020);
    }

    #[test]
    fn reads_data_directories() {
        let image = build_pe(true);
        let pe = PeFile::parse(&image).unwrap();
        let export = pe
            .data_directory(DIRECTORY_ENTRY_EXPORT)
            .expect("export directory present");
        assert_ne!(export.virtual_address, 0);
        assert_ne!(export.size, 0);
        // Declared NumberOfRvaAndSizes is 16, so index 16 is out of range.
        assert_eq!(pe.data_directory(16), None);
    }

    #[test]
    fn translates_rva_to_file_offset() {
        let image = build_pe(true);
        let pe = PeFile::parse(&image).unwrap();
        // The first byte of the section maps to its raw pointer.
        assert_eq!(pe.rva_to_offset(SECTION_VA), Some(SECTION_RAW));
        // An RVA before any section (in the headers) has no section-backed
        // file offset via this translation.
        assert_eq!(pe.rva_to_offset(0), None);
        // An RVA past the section is unmapped.
        assert_eq!(pe.rva_to_offset(SECTION_VA + SECTION_SIZE as u32), None);
    }

    #[test]
    fn enumerates_exports_with_ordinals() {
        let image = build_pe(true);
        let pe = PeFile::parse(&image).unwrap();
        let exports: Vec<_> = pe.exports().expect("image has exports").collect();
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].name, "ExportedFn");
        // Ordinal base 5 + name-ordinal-table index 0.
        assert_eq!(exports[0].ordinal, 5);
    }

    #[test]
    fn enumerates_imported_modules() {
        let image = build_pe(true);
        let pe = PeFile::parse(&image).unwrap();
        let imports: Vec<_> = pe.imports().expect("image has imports").collect();
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].name, "OTHER.dll");
    }

    #[test]
    fn rejects_a_buffer_too_small_for_a_dos_header() {
        assert_eq!(PeFile::parse(&[0u8; 16]).unwrap_err(), PeError::TooSmall);
    }

    #[test]
    fn rejects_a_bad_dos_signature() {
        let mut image = build_pe(true);
        image[0] = 0;
        assert_eq!(PeFile::parse(&image).unwrap_err(), PeError::BadDosSignature);
    }

    #[test]
    fn rejects_a_bad_pe_signature() {
        let mut image = build_pe(true);
        image[PE_OFF] = 0; // corrupt "PE\0\0"
        assert_eq!(PeFile::parse(&image).unwrap_err(), PeError::BadPeSignature);
    }

    #[test]
    fn rejects_an_unknown_optional_magic() {
        let mut image = build_pe(true);
        let opt = PE_OFF + 4 + 20;
        image[opt] = 0xff;
        image[opt + 1] = 0xff;
        assert_eq!(
            PeFile::parse(&image).unwrap_err(),
            PeError::BadOptionalMagic(0xffff)
        );
    }

    #[test]
    fn an_image_without_exports_returns_none() {
        // A pristine image advertises exports...
        let with_exports = build_pe(true);
        assert!(PeFile::parse(&with_exports).unwrap().exports().is_some());

        // ...but zeroing its export data-directory entry makes `exports`
        // report none.
        let mut without = build_pe(true);
        let opt = PE_OFF + 4 + 20;
        let data_dirs = opt + 112;
        without[data_dirs..data_dirs + 8].fill(0);
        assert!(PeFile::parse(&without).unwrap().exports().is_none());
    }
}
