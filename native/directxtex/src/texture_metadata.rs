use crate::{
    DDS_FLAGS, DDSMetaData, DXGI_FORMAT, Result, TEX_ALPHA_MODE, TEX_DIMENSION, TEX_MISC_FLAG,
    TEX_MISC_FLAG2, TGA_FLAGS,
    ffi::{self, prelude::*},
};
use core::ptr;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(C)]
pub struct TexMetadata {
    pub width: usize,
    /// Should be 1 for 1D textures
    pub height: usize,
    /// Should be 1 for 1D or 2D textures
    pub depth: usize,
    /// For cubemap, this is a multiple of 6
    pub array_size: usize,
    pub mip_levels: usize,
    pub misc_flags: u32,
    pub misc_flags2: u32,
    pub format: DXGI_FORMAT,
    pub dimension: TEX_DIMENSION,
}

impl TexMetadata {
    #[must_use]
    /// Returns `None` to indicate an out-of-range error
    pub fn compute_index(&self, mip: usize, item: usize, slice: usize) -> Option<usize> {
        let result =
            unsafe { ffi::DirectXTexFFI_TexMetadata_ComputIndex(self.into(), mip, item, slice) };
        (result != usize::MAX).then_some(result)
    }

    #[must_use]
    /// Helper for [`misc_flags`](Self::misc_flags)
    pub fn is_cubemap(&self) -> bool {
        (self.misc_flags & TEX_MISC_FLAG::TEX_MISC_TEXTURECUBE.bits()) != 0
    }

    #[must_use]
    /// Helpers for [`misc_flags2`](Self::misc_flags2)
    pub fn is_pm_alpha(&self) -> bool {
        (self.misc_flags2 & TEX_MISC_FLAG2::TEX_MISC2_ALPHA_MODE_MASK.bits())
            == TEX_ALPHA_MODE::TEX_ALPHA_MODE_PREMULTIPLIED.bits()
    }

    /// Helpers for [`misc_flags2`](Self::misc_flags2)
    pub fn set_alpha_mode(&mut self, mode: TEX_ALPHA_MODE) {
        self.misc_flags2 =
            (self.misc_flags2 & !TEX_MISC_FLAG2::TEX_MISC2_ALPHA_MODE_MASK.bits()) | mode.bits();
    }

    #[must_use]
    /// Helpers for [`misc_flags2`](Self::misc_flags2)
    pub fn get_alpha_mode(&self) -> TEX_ALPHA_MODE {
        (self.misc_flags2 & TEX_MISC_FLAG2::TEX_MISC2_ALPHA_MODE_MASK.bits()).into()
    }

    #[must_use]
    /// Helper for [`dimension`](Self::dimension)
    pub fn is_volumemap(&self) -> bool {
        self.dimension == TEX_DIMENSION::TEX_DIMENSION_TEXTURE3D
    }

    pub fn from_dds(
        source: &[u8],
        flags: DDS_FLAGS,
        dd_pixel_format: Option<&mut DDSMetaData>,
    ) -> Result<Self> {
        let mut result = Self::default();
        let hr = unsafe {
            ffi::DirectXTexFFI_GetMetadataFromDDSMemoryEx(
                source.as_ffi_ptr(),
                source.len(),
                flags,
                (&mut result).into(),
                dd_pixel_format.into_ffi_ptr(),
            )
        };
        hr.success(result)
    }

    pub fn from_hdr(source: &[u8]) -> Result<Self> {
        let mut result = Self::default();
        let hr = unsafe {
            ffi::DirectXTexFFI_GetMetadataFromHDRMemory(
                source.as_ffi_ptr(),
                source.len(),
                (&mut result).into(),
            )
        };
        hr.success(result)
    }

    pub fn from_tga(source: &[u8], flags: TGA_FLAGS) -> Result<Self> {
        let mut result = Self::default();
        let hr = unsafe {
            ffi::DirectXTexFFI_GetMetadataFromTGAMemory(
                source.as_ffi_ptr(),
                source.len(),
                flags,
                (&mut result).into(),
            )
        };
        hr.success(result)
    }

    pub fn encode_dds_header(&self, flags: DDS_FLAGS) -> Result<Box<[u8]>> {
        let mut required = 0;
        let hr = unsafe {
            ffi::DirectXTexFFI_EncodeDDSHeader(
                self.into(),
                flags,
                ptr::null_mut(),
                0,
                (&mut required).into(),
            )
        };
        hr.success(())?;

        let mut v = Vec::new();
        v.resize_with(required, Default::default);
        let hr = unsafe {
            ffi::DirectXTexFFI_EncodeDDSHeader(
                self.into(),
                flags,
                v.as_mut_ffi_ptr(),
                v.len(),
                (&mut required).into(),
            )
        };
        hr.success(()).map(|()| v.into_boxed_slice())
    }
}

#[cfg(test)]
mod tests {
    use crate::TexMetadata;
    use std::fs;

    #[test]
    fn encode_dds_header() {
        let file = fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/ferris_wheel.dds"
        ))
        .unwrap();
        let tex = TexMetadata::from_dds(&file, Default::default(), None).unwrap();
        let header = tex.encode_dds_header(Default::default()).unwrap();
        let len = header.len();
        assert_eq!(&header[..], &file[..len]);
    }
}
