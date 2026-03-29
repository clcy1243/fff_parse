#!/usr/bin/env python3
"""
Phase 4: Debug and fix the thumbnail curve extraction.
Key bug: forward-fill from 0 for leading empty bins.
"""
import numpy as np
import tifffile

CORRECTIONS = {
    "test2": {
        "film_type": 1, "gamma": 2.0, "ev": 1.0, "saturation": 25,
        "shadow": [0, 0, 0, 0], "gray": [128, 94, 92, 90],
        "highlight": [16192, 16192, 16192, 16192],
        "dot_color": [0,0,0,0,0,0,0, 255,255,255,255,255,255,255],
    },
    "test3": {
        "film_type": 1, "gamma": 2.0, "ev": 1.0, "saturation": 25,
        "shadow": [2496, 2496, 5376, 8640], "gray": [128, 94, 92, 90],
        "highlight": [16192, 16192, 16192, 16192],
        "dot_color": [0,0,0,0,0,0,0, 255,255,255,255,255,255,255],
    },
    "test1": {
        "film_type": 1, "gamma": 2.0, "ev": 1.0, "saturation": 15,
        "shadow": [448, 448, 4352, 8832], "gray": [128, 134, 131, 108],
        "highlight": [15680, 11520, 14720, 15680],
        "dot_color": [0,0,0,0,0,0,0, 255,255,255,255,255,255,255],
    },
}

BASE = "/Users/will/vmwareShare/test_image"


def step_invert(raw, corr):
    result = np.copy(raw)
    for ch in range(3):
        hi = corr["highlight"][ch + 1] * 4.0
        if hi > 0:
            result[:,:,ch] = ((hi - raw[:,:,ch]).clip(0) * (65535.0 / hi)).clip(0, 65535)
    return result

def step_levels(data, corr):
    result = np.empty_like(data)
    for ch in range(3):
        bl = corr["shadow"][ch + 1] * 4.0 / 65535.0
        wh = corr["highlight"][ch + 1] * 4.0 / 65535.0
        rng = max(wh - bl, 0.001)
        result[:,:,ch] = ((data[:,:,ch] / 65535.0 - bl) / rng).clip(0, 1) * 65535.0
    return result

def step_per_ch_gamma(data, corr):
    result = np.empty_like(data)
    for ch in range(3):
        gamma_c = max(corr["gray"][ch + 1] / 128.0, 0.01)
        result[:,:,ch] = np.power((data[:,:,ch] / 65535.0).clip(0, 1), 1.0 / gamma_c) * 65535.0
    return result

def step_master_gamma(data, corr):
    gamma_m = max(corr["gamma"] - 1.0, 0.01)
    return np.power((data / 65535.0).clip(0, 1), 1.0 / gamma_m) * 65535.0

def step_saturation(data, corr):
    sat = corr["saturation"] / 100.0
    if abs(sat) < 0.001:
        return data
    v = data / 65535.0
    lum = 0.2126 * v[:,:,0] + 0.7152 * v[:,:,1] + 0.0722 * v[:,:,2]
    result = np.empty_like(v)
    for ch in range(3):
        result[:,:,ch] = (lum + (v[:,:,ch] - lum) * (1 + sat)).clip(0, 1)
    return result * 65535.0

def apply_curve(data, curves):
    result = np.empty_like(data)
    for ch in range(3):
        idx = data[:,:,ch].astype(np.int32).clip(0, 65535)
        result[:,:,ch] = curves[ch][idx]
    return result

def mae(a, b):
    return np.mean(np.abs(a - b))

def mae_per_ch(a, b):
    return [np.mean(np.abs(a[:,:,c] - b[:,:,c])) for c in range(3)]


def extract_thumb_buggy(fff_path, corr, n_bins=16384):
    """Original Rust algorithm (forward-fill from 0)."""
    tif = tifffile.TiffFile(fff_path)
    thumb_8 = tif.pages[1].asarray().astype(np.float32)
    preview_16 = tif.pages[2].asarray().astype(np.float32)
    
    gamma_m = max(corr["gamma"] - 1.0, 0.01)
    sat = corr["saturation"] / 100.0
    dc = corr["dot_color"]
    out_lo = dc[0] / 255.0
    out_hi = dc[7] / 255.0
    out_range = max(out_hi - out_lo, 0.001)
    
    # Invert preview
    inv = np.empty_like(preview_16)
    for ch in range(3):
        hi = corr["highlight"][ch + 1] * 4.0
        if hi > 0:
            inv[:,:,ch] = ((hi - preview_16[:,:,ch]).clip(0) * (65535.0 / hi)).clip(0, 65535)
    
    # Reverse-process thumbnail
    rgb = thumb_8 / 255.0
    if abs(sat) > 0.001:
        lum = 0.2126 * rgb[:,:,0] + 0.7152 * rgb[:,:,1] + 0.0722 * rgb[:,:,2]
        for ch in range(3):
            rgb[:,:,ch] = lum + (rgb[:,:,ch] - lum) / (1.0 + sat)
    
    for ch in range(3):
        rgb[:,:,ch] = ((rgb[:,:,ch] - out_lo) / out_range).clip(0, 1)
    rgb = np.power(rgb.clip(0, 1), gamma_m)
    for ch in range(3):
        gamma_c = max(corr["gray"][ch + 1] / 128.0, 0.01)
        rgb[:,:,ch] = np.power(rgb[:,:,ch].clip(0, 1), gamma_c)
    for ch in range(3):
        bl = corr["shadow"][ch + 1] * 4.0 / 65535.0
        wh = corr["highlight"][ch + 1] * 4.0 / 65535.0
        rng = max(wh - bl, 0.001)
        rgb[:,:,ch] = (rgb[:,:,ch] * rng + bl).clip(0, 1)
    
    target_16 = (rgb * 65535.0).clip(0, 65535)
    
    # Build curve - BUGGY version (forward-fill from 0)
    BINS = n_bins
    curves = []
    for ch in range(3):
        inv_ch = inv[:,:,ch].flatten()
        tgt_ch = target_16[:,:,ch].flatten()
        
        sums = np.zeros(BINS, dtype=np.float64)
        counts = np.zeros(BINS, dtype=np.int64)
        bin_idx = (inv_ch / 65535.0 * (BINS - 1)).astype(np.int32).clip(0, BINS - 1)
        np.add.at(sums, bin_idx, tgt_ch.astype(np.float64))
        np.add.at(counts, bin_idx, 1)
        
        bin_avgs = np.zeros(BINS, dtype=np.float64)
        last_valid = 0.0
        for i in range(BINS):
            if counts[i] > 0:
                bin_avgs[i] = sums[i] / counts[i]
                last_valid = bin_avgs[i]
            else:
                bin_avgs[i] = last_valid
        
        for i in range(1, BINS):
            if bin_avgs[i] < bin_avgs[i-1]:
                bin_avgs[i] = bin_avgs[i-1]
        
        lut = np.zeros(65536, dtype=np.float64)
        for i in range(65536):
            pos = i / 65535.0 * (BINS - 1)
            lo = min(int(pos), BINS - 2)
            frac = pos - lo
            lut[i] = bin_avgs[lo] * (1 - frac) + bin_avgs[lo + 1] * frac
        
        curves.append(lut)
    
    return curves


def extract_thumb_fixed(fff_path, corr, n_bins=4096):
    """Fixed algorithm: use np.interp for gap filling, reduced bins."""
    tif = tifffile.TiffFile(fff_path)
    thumb_8 = tif.pages[1].asarray().astype(np.float32)
    preview_16 = tif.pages[2].asarray().astype(np.float32)
    
    gamma_m = max(corr["gamma"] - 1.0, 0.01)
    sat = corr["saturation"] / 100.0
    dc = corr["dot_color"]
    out_lo = dc[0] / 255.0
    out_hi = dc[7] / 255.0
    out_range = max(out_hi - out_lo, 0.001)
    
    inv = np.empty_like(preview_16)
    for ch in range(3):
        hi = corr["highlight"][ch + 1] * 4.0
        if hi > 0:
            inv[:,:,ch] = ((hi - preview_16[:,:,ch]).clip(0) * (65535.0 / hi)).clip(0, 65535)
    
    rgb = thumb_8 / 255.0
    if abs(sat) > 0.001:
        lum = 0.2126 * rgb[:,:,0] + 0.7152 * rgb[:,:,1] + 0.0722 * rgb[:,:,2]
        for ch in range(3):
            rgb[:,:,ch] = lum + (rgb[:,:,ch] - lum) / (1.0 + sat)
    
    for ch in range(3):
        rgb[:,:,ch] = ((rgb[:,:,ch] - out_lo) / out_range).clip(0, 1)
    rgb = np.power(rgb.clip(0, 1), gamma_m)
    for ch in range(3):
        gamma_c = max(corr["gray"][ch + 1] / 128.0, 0.01)
        rgb[:,:,ch] = np.power(rgb[:,:,ch].clip(0, 1), gamma_c)
    for ch in range(3):
        bl = corr["shadow"][ch + 1] * 4.0 / 65535.0
        wh = corr["highlight"][ch + 1] * 4.0 / 65535.0
        rng = max(wh - bl, 0.001)
        rgb[:,:,ch] = (rgb[:,:,ch] * rng + bl).clip(0, 1)
    
    target_16 = (rgb * 65535.0).clip(0, 65535)
    
    # Build curve - FIXED: use np.interp for ALL gap filling
    BINS = n_bins
    curves = []
    for ch in range(3):
        ch_name = ["R", "G", "B"][ch]
        inv_ch = inv[:,:,ch].flatten()
        tgt_ch = target_16[:,:,ch].flatten()
        
        sums = np.zeros(BINS, dtype=np.float64)
        counts = np.zeros(BINS, dtype=np.int64)
        bin_idx = (inv_ch / 65535.0 * (BINS - 1)).astype(np.int32).clip(0, BINS - 1)
        np.add.at(sums, bin_idx, tgt_ch.astype(np.float64))
        np.add.at(counts, bin_idx, 1)
        
        # Debug: show bin population
        populated = np.sum(counts > 0)
        total_pixels = len(inv_ch)
        print(f"    {ch_name}: {populated}/{BINS} bins populated ({total_pixels} pixels, avg {total_pixels/max(populated,1):.1f}/bin)")
        
        # Compute bin averages for populated bins
        bin_avgs = np.zeros(BINS, dtype=np.float64)
        mask = counts > 0
        bin_avgs[mask] = sums[mask] / counts[mask]
        
        # Use np.interp to fill ALL gaps (including leading/trailing)
        valid_idx = np.where(mask)[0]
        if len(valid_idx) >= 2:
            bin_avgs = np.interp(np.arange(BINS), valid_idx, bin_avgs[valid_idx])
        elif len(valid_idx) == 1:
            bin_avgs[:] = bin_avgs[valid_idx[0]]
        
        # Enforce monotonicity
        for i in range(1, BINS):
            if bin_avgs[i] < bin_avgs[i-1]:
                bin_avgs[i] = bin_avgs[i-1]
        
        # Interpolate to 65536
        lut = np.interp(np.arange(65536), 
                        np.arange(BINS) * 65535.0 / (BINS - 1),
                        bin_avgs)
        
        curves.append(lut)
    
    return curves


def full_pipeline(raw, corr, curves):
    data = step_invert(raw, corr)
    data = apply_curve(data, curves)
    data = step_levels(data, corr)
    data = step_per_ch_gamma(data, corr)
    data = step_master_gamma(data, corr)
    data = step_saturation(data, corr)
    return data


def main():
    for name in ["test2", "test3", "test1"]:
        corr = CORRECTIONS[name]
        fff_path = f"{BASE}/{name}.fff"
        tif_path = f"{BASE}/{name}.tif"
        
        print(f"\n{'='*70}")
        print(f"  {name}: Shadow={corr['shadow']}")
        print(f"{'='*70}")
        
        raw = tifffile.imread(fff_path, key=0).astype(np.float32)
        ref = tifffile.imread(tif_path, key=0).astype(np.float32)
        
        # Method A: Original buggy extraction (16384 bins, forward-fill from 0)
        print(f"\n  A) Buggy (16384 bins, forward-fill):")
        curves_a = extract_thumb_buggy(fff_path, corr, n_bins=16384)
        result_a = full_pipeline(raw, corr, curves_a)
        m_a = mae(result_a, ref)
        mpc_a = mae_per_ch(result_a, ref)
        print(f"    MAE: {m_a:.2f}  R={mpc_a[0]:.2f} G={mpc_a[1]:.2f} B={mpc_a[2]:.2f}")
        
        # Method B: Fixed extraction (4096 bins, np.interp)
        print(f"\n  B) Fixed (4096 bins, interp):")
        curves_b = extract_thumb_fixed(fff_path, corr, n_bins=4096)
        result_b = full_pipeline(raw, corr, curves_b)
        m_b = mae(result_b, ref)
        mpc_b = mae_per_ch(result_b, ref)
        print(f"    MAE: {m_b:.2f}  R={mpc_b[0]:.2f} G={mpc_b[1]:.2f} B={mpc_b[2]:.2f}")
        
        # Method C: Fixed (1024 bins)
        print(f"\n  C) Fixed (1024 bins, interp):")
        curves_c = extract_thumb_fixed(fff_path, corr, n_bins=1024)
        result_c = full_pipeline(raw, corr, curves_c)
        m_c = mae(result_c, ref)
        mpc_c = mae_per_ch(result_c, ref)
        print(f"    MAE: {m_c:.2f}  R={mpc_c[0]:.2f} G={mpc_c[1]:.2f} B={mpc_c[2]:.2f}")
        
        # Method D: Fixed (512 bins)
        print(f"\n  D) Fixed (512 bins, interp):")
        curves_d = extract_thumb_fixed(fff_path, corr, n_bins=512)
        result_d = full_pipeline(raw, corr, curves_d)
        m_d = mae(result_d, ref)
        mpc_d = mae_per_ch(result_d, ref)
        print(f"    MAE: {m_d:.2f}  R={mpc_d[0]:.2f} G={mpc_d[1]:.2f} B={mpc_d[2]:.2f}")
        
        print(f"\n  Summary: A={m_a:.0f} B={m_b:.0f} C={m_c:.0f} D={m_d:.0f}")


if __name__ == "__main__":
    main()
