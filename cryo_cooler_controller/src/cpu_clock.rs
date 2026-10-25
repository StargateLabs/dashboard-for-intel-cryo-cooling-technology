//! Current highest core clock from read-only Core Temp shared memory.
//! Core Temp layout: https://www.alcpu.com/CoreTemp/developers.html (version 2).

#[derive(Debug, Clone, Copy)]
pub struct Clock {
    pub mhz: f32,
    pub source: &'static str,
}

fn valid(mhz: f32) -> bool { mhz.is_finite() && (100.0..=10_000.0).contains(&mhz) }

fn core_temp_max(bytes: &[u8]) -> Option<f32> {
    let u32_at = |offset| Some(u32::from_le_bytes(bytes.get(offset..offset+4)?.try_into().ok()?));
    let f32_at = |offset| Some(f32::from_le_bytes(bytes.get(offset..offset+4)?.try_into().ok()?));
    // 4-byte packing, exact v2 offsets from the provider's published structure.
    if u32_at(2688)? < 2 { return None; }
    let count = u32_at(1536)?.checked_mul(u32_at(1540)?)? as usize;
    if !(1..=256).contains(&count) { return None; }
    let fsb = f32_at(2576)?;
    if !fsb.is_finite() || !(10.0..=1000.0).contains(&fsb) { return None; }
    bytes.get(..3716 + count*4)?;
    (0..count).filter_map(|i| f32_at(3716+i*4)).map(|m| m*fsb)
        .filter(|v| valid(*v)).max_by(f32::total_cmp)
}

pub fn read() -> Option<Clock> {
    // Each call replaces the previous sample. Never retain a historical peak.
    shared_bytes("CoreTempMappingObjectEx", 4740)
        .or_else(|| shared_bytes("Global\\CoreTempMappingObjectEx", 4740))
        .and_then(|b| core_temp_max(&b)).map(|mhz| Clock { mhz, source: "Core Temp" })
}

#[cfg(windows)]
fn shared_bytes(name: &str, maximum: usize) -> Option<Vec<u8>> {
    use windows_sys::Win32::{Foundation::CloseHandle, System::Memory::*};
    #[link(name="kernel32")]
    extern "system" { fn VirtualQuery(address: *const core::ffi::c_void, info: *mut MEMORY_BASIC_INFORMATION, length: usize) -> usize; }
    let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let handle = OpenFileMappingW(FILE_MAP_READ, 0, name.as_ptr());
        if handle == 0 { return None; }
        let view = MapViewOfFile(handle, FILE_MAP_READ, 0, 0, 0);
        if view == 0 { CloseHandle(handle); return None; }
        let mut info: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
        let result = if VirtualQuery(view as *const _, &mut info, std::mem::size_of_val(&info)) == 0
            || info.State != MEM_COMMIT || info.Protect & (PAGE_NOACCESS | PAGE_GUARD) != 0 {
            None
        } else {
            (info.BaseAddress as usize).checked_add(info.RegionSize)
                .and_then(|end| end.checked_sub(view as usize))
                .filter(|len| *len > 0)
                .map(|len| std::slice::from_raw_parts(view as *const u8,len.min(maximum)).to_vec())
        };
        UnmapViewOfFile(view);
        CloseHandle(handle);
        result
    }
}

#[cfg(not(windows))]
fn shared_bytes(_: &str, _: usize) -> Option<Vec<u8>> { None }

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot() -> Vec<u8> {
        let mut b=vec![0;4740];
        for (offset,value) in [(1536,24_u32),(1540,1),(2688,2)] { b[offset..offset+4].copy_from_slice(&value.to_le_bytes()); }
        b[2576..2580].copy_from_slice(&100_f32.to_le_bytes());
        b[3716..3720].copy_from_slice(&62_f32.to_le_bytes());
        b[3720..3724].copy_from_slice(&64_f32.to_le_bytes());
        b
    }
    #[test] fn core_temp_uses_per_core_multipliers_and_drops_old_peaks() {
        let mut b=snapshot();
        assert_eq!(core_temp_max(&b),Some(6400.0));
        b[3716..3724].fill(0);
        b[3716..3720].copy_from_slice(&8_f32.to_le_bytes());
        assert_eq!(core_temp_max(&b),Some(800.0));
        assert_eq!(core_temp_max(&[]),None);
        assert_eq!(core_temp_max(&b[..3717]),None);
    }
    #[test] fn core_temp_rejects_unsupported_layout_and_invalid_bus_clock() {
        let mut b=snapshot(); b[2688..2692].copy_from_slice(&1_u32.to_le_bytes());
        assert_eq!(core_temp_max(&b),None);
        let mut b=snapshot(); b[2576..2580].copy_from_slice(&f32::NAN.to_le_bytes());
        assert_eq!(core_temp_max(&b),None);
    }
}
