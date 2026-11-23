use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::null_mut;

use windows_sys::Win32::Security::WinTrust::{
    WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO,
    WTD_CHOICE_FILE, WTD_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT, WTD_STATEACTION_CLOSE,
    WTD_STATEACTION_VERIFY, WTD_UI_NONE, WTHelperGetProvCertFromChain,
    WTHelperGetProvSignerFromChain, WTHelperProvDataFromStateData, WinVerifyTrustEx,
};

pub(super) fn verify_publisher(installed: &Path, candidate: &Path) -> Result<(), String> {
    let installed = publisher(installed)
        .map_err(|error| format!("Could not verify the installed executable: {error}"))?;
    let candidate = publisher(candidate)
        .map_err(|error| format!("Could not verify the downloaded executable: {error}"))?;
    if installed != candidate {
        return Err("The executable was signed by a different publisher.".to_owned());
    }
    Ok(())
}

fn publisher(path: &Path) -> Result<Vec<u8>, String> {
    let path: Vec<_> = path.as_os_str().encode_wide().chain([0]).collect();
    let mut file = WINTRUST_FILE_INFO {
        cbStruct: size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: path.as_ptr(),
        ..Default::default()
    };
    let mut policy = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    let mut trust = WINTRUST_DATA {
        cbStruct: size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 { pFile: &mut file },
        dwStateAction: WTD_STATEACTION_VERIFY,
        dwProvFlags: WTD_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT,
        ..Default::default()
    };

    // The path and file info outlive the verification state. Its certificate pointers remain
    // valid until CLOSE, so copy the publisher before closing, including on a failed check.
    unsafe {
        let status = WinVerifyTrustEx(null_mut(), &mut policy, &mut trust);
        let subject = if status == 0 {
            (|| {
                let data = WTHelperProvDataFromStateData(trust.hWVTStateData);
                if data.is_null() {
                    return None;
                }
                let signer = WTHelperGetProvSignerFromChain(data, 0, 0, 0);
                if signer.is_null() {
                    return None;
                }
                let leaf = WTHelperGetProvCertFromChain(signer, 0).as_ref()?;
                let subject = leaf.pCert.as_ref()?.pCertInfo.as_ref()?.Subject;
                (!subject.pbData.is_null() && subject.cbData > 0).then(|| {
                    std::slice::from_raw_parts(subject.pbData, subject.cbData as usize).to_vec()
                })
            })()
            .ok_or_else(|| "The verified signature has no publisher.".to_owned())
        } else {
            Err(format!(
                "Windows rejected the signature (0x{:08X}).",
                status as u32
            ))
        };
        trust.dwStateAction = WTD_STATEACTION_CLOSE;
        WinVerifyTrustEx(null_mut(), &mut policy, &mut trust);
        subject
    }
}
