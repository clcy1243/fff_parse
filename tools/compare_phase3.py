#!/usr/bin/env python3
"""
Phase 3: Compare 8-bit thumbnail curve extraction vs 16-bit TIF ground truth.
Test our exact Rust pipeline simulation and identify remaining gaps.
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
        v = ((data[:,:,ch] / 65535.0 - bl) / rng).clip(0, 1)
        result[:,:,ch] = v * 65535.0
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


def step_saturation(data, corr, weights="709"):
    sat = corr["saturation"] / 100.0
    if abs(sat) < 0.001:
        return data
    v = data / 65535.0
    if weights == "709":
        lum = 0.2126 * v[:,:,0] + 0.7152 * v[:,:,1] + 0.0722 * v[:,:,2]
    else:
        lum = 0.299 * v[:,:,0] + 0.587 * v[:,:,1] + 0.114 * v[:,:,2]
    result = np.empty_like(v)
    for ch in range(3):
        result[:,:,ch] = (lum + (v[:,:,ch] - lum) * (1 + sat)).clip(0, 1)
    return result * 65535.0


def extract_curve_65k(src, tgt):
    """Extract 65536-point per-channel curve."""
    BINS = 65536
    curves = []
    for ch in range(3):
        s = src[:,:,ch].flatten().astype(np.int32).clip(0, 65535)
        t = tgt[:,:,ch].flatten().astype(np.float64)
        
        sums = np.zeros(BINS, dtype=np.float64)
        counts = np.zeros(BINS, dtype=np.int64)
        np.add.at(sums, s, t)
        np.add.at(counts, s, 1)
        
        curve = np.zeros(BINS, dtype=np.float64)
        mask = counts > 0
        curve[mask] = sums[mask] / counts[mask]
        
        valid_idx = np.where(mask)[0]
        if len(valid_idx) > 2:
            curve = np.interp(np.arange(BINS), valid_idx, curve[valid_idx])
        
        for i in range(1, BINS):
            if curve[i] < curve[i-1]:
                curve[i] = curve[i-1]
        
        curves.append(curve)
    return curves


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


def extract_from_thumbnail(fff_path, corr):
    """Simulate Rust extract_film_curve: use IFD#1 (8-bit) + IFD#2 (16-bit preview)."""
    tif = tifffile.TiffFile(fff_path)
    thumb_8 = tif.pages[1].asarray()  # IFD#1: 8-bit thumbnail
    preview_16 = tif.pages[2].asarray()  # IFD#2: 16-bit preview
    print(f"  Thumbnail: {thumb_8.shape} {thumb_8.dtype}")
    print(f"  Preview16: {preview_16.shape} {preview_16.dtype}")
    
    thumb = thumb_8.astype(np.float32)
    prev = preview_16.astype(np.float32)
    
    # Invert preview
    inv = np.empty_like(prev)
    for ch in range(3):
        hi = corr["highlight"][ch + 1] * 4.0
        if hi > 0:
            inv[:,:,ch] = ((hi - prev[:,:,ch]).clip(0) * (65535.0 / hi)).clip(0, 65535)
    
    # Reverse-process thumbnail (same as Rust extract_film_curve)
    gamma_m = max(corr["gamma"] - 1.0, 0.01)
    sat = corr["saturation"] / 100.0
    dc = corr["dot_color"]
    out_lo = dc[0] / 255.0
    out_hi = dc[7] / 255.0
    out_range = max(out_hi - out_lo, 0.001)
    
    rgb = thumb / 255.0  # 8-bit → [0,1]
    
    # Un-saturation (BT.709)
    if abs(sat) > 0.001:
        lum = 0.2126 * rgb[:,:,0] + 0.7152 * rgb[:,:,1] + 0.0722 * rgb[:,:,2]
        for ch in range(3):
            rgb[:,:,ch] = lum + (rgb[:,:,ch] - lum) / (1.0 + sat)
    
    # Un-exposure (ev=1 → no change)
    ev = corr["ev"]
    if abs(ev - 1.0) > 0.001:
        rgb /= 2.0 ** (ev - 1.0)
    
    # Un-output-levels
    for ch in range(3):
        rgb[:,:,ch] = ((rgb[:,:,ch] - out_lo) / out_range).clip(0, 1)
    
    # Un-master-gamma
    rgb = np.power(rgb.clip(0, 1), gamma_m)
    
    # Un-per-channel-gamma
    for ch in range(3):
        gamma_c = max(corr["gray"][ch + 1] / 128.0, 0.01)
        rgb[:,:,ch] = np.power(rgb[:,:,ch].clip(0, 1), gamma_c)
    
    # Un-levels
    for ch in range(3):
        bl = corr["shadow"][ch + 1] * 4.0 / 65535.0
        wh = corr["highlight"][ch + 1] * 4.0 / 65535.0
        rng = max(wh - bl, 0.001)
        rgb[:,:,ch] = (rgb[:,:,ch] * rng + bl).clip(0, 1)
    
    target_16 = (rgb * 65535.0).clip(0, 65535)
    
    # Build curve from thumbnail resolution
    BINS = 16384  # Same as Rust
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
        
        # Enforce monotonicity
        for i in range(1, BINS):
            if bin_avgs[i] < bin_avgs[i-1]:
                bin_avgs[i] = bin_avgs[i-1]
        
        # Interpolate to 65536
        lut = np.zeros(65536, dtype=np.float64)
        for i in range(65536):
            pos = i / 65535.0 * (BINS - 1)
            lo = min(int(pos), BINS - 2)
            frac = pos - lo
            lut[i] = bin_avgs[lo] * (1 - frac) + bin_avgs[lo + 1] * frac
        
        curves.append(lut)
    
    return curves


def extract_from_tif(fff_path, tif_path, corr):
    """Extract curve using full-resolution 16-bit TIF as reference."""
    raw = tifffile.imread(fff_path, key=0).astype(np.float32)
    ref = tifffile.imread(tif_path, key=0).astype(np.float32)
    
    inverted = step_invert(raw, corr)
    
    # Reverse-process TIF
    gamma_m = max(corr["gamma"] - 1.0, 0.01)
    sat = corr["saturation"] / 100.0
    
    v = ref / 65535.0
    
    # Un-saturation
    if abs(sat) > 0.001:
        lum = 0.2126 * v[:,:,0] + 0.7152 * v[:,:,1] + 0.0722 * v[:,:,2]
        for ch in range(3):
            v[:,:,ch] = lum + (v[:,:,ch] - lum) / (1 + sat)
    
    # Un-master-gamma
    v = np.power(v.clip(0, 1), gamma_m)
    
    # Un-per-channel-gamma
    for ch in range(3):
        gamma_c = max(corr["gray"][ch + 1] / 128.0, 0.01)
        v[:,:,ch] = np.power(v[:,:,ch].clip(0, 1), gamma_c)
    
    # Un-levels
    for ch in range(3):
        bl = corr["shadow"][ch + 1] * 4.0 / 65535.0
        wh = corr["highlight"][ch + 1] * 4.0 / 65535.0
        rng = max(wh - bl, 0.001)
        v[:,:,ch] = (v[:,:,ch] * rng + bl).clip(0, 1)
    
    target = (v * 65535.0).clip(0, 65535)
    
    return extract_curve_65k(inverted, target)


def full_pipeline(raw, corr, curves):
    """Apply full pipeline: invert → curve → levels → gamma → sat"""
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
        print(f"  {name}")
        print(f"{'='*70}")
        
        raw = tifffile.imread(fff_path, key=0).astype(np.float32)
        ref = tifffile.imread(tif_path, key=0).astype(np.float32)
        print(f"  Full-res: {raw.shape}")
        
        # Method A: 8-bit thumbnail extraction (our current Rust approach)
        print(f"\n  --- Method A: 8-bit thumbnail extraction ---")
        curves_thumb = extract_from_thumbnail(fff_path, corr)
        result_a = full_pipeline(raw, corr, curves_thumb)
        m_a = mae(result_a, ref)
        mpc_a = mae_per_ch(result_a, ref)
        print(f"  MAE: {m_a:.2f}  R={mpc_a[0]:.2f} G={mpc_a[1]:.2f} B={mpc_a[2]:.2f}")
        
        # Method B: 16-bit TIF extraction (ground truth)
        print(f"\n  --- Method B: 16-bit TIF extraction ---")
        curves_tif = extract_from_tif(fff_path, tif_path, corr)
        result_b = full_pipeline(raw, corr, curves_tif)
        m_b = mae(result_b, ref)
        mpc_b = mae_per_ch(result_b, ref)
        print(f"  MAE: {m_b:.2f}  R={mpc_b[0]:.2f} G={mpc_b[1]:.2f} B={mpc_b[2]:.2f}")
        
        # Method C: No film curve at all
        print(f"\n  --- Method C: No film curve ---")
        data_c = step_invert(raw, corr)
        data_c = step_levels(data_c, corr)
        data_c = step_per_ch_gamma(data_c, corr)
        data_c = step_master_gamma(data_c, corr)
        data_c = step_saturation(data_c, corr)
        m_c = mae(data_c, ref)
        mpc_c = mae_per_ch(data_c, ref)
        print(f"  MAE: {m_c:.2f}  R={mpc_c[0]:.2f} G={mpc_c[1]:.2f} B={mpc_c[2]:.2f}")
        
        # Compare curves at key points
        print(f"\n  --- Curve comparison: thumbnail vs TIF ---")
        for ch, ch_name in enumerate(["R", "G", "B"]):
            ct = np.array(curves_thumb[ch])
            cr = np.array(curves_tif[ch])
            diff = np.abs(ct - cr)
            print(f"  {ch_name}: MAE={np.mean(diff):.2f}, max={np.max(diff):.2f}")
            for i in [0, 8192, 16384, 32768, 49152, 65535]:
                print(f"    [{i:5d}] thumb={ct[i]:8.1f} tif={cr[i]:8.1f} Δ={ct[i]-cr[i]:+8.1f}")
        
        # Show percentage of full range
        print(f"\n  --- Summary ---")
        print(f"  Method A (thumbnail): MAE={m_a:.2f} ({m_a/65535*100:.3f}%)")
        print(f"  Method B (TIF):       MAE={m_b:.2f} ({m_b/65535*100:.3f}%)")
        print(f"  Method C (no curve):  MAE={m_c:.2f} ({m_c/65535*100:.3f}%)")
        print(f"  Improvement A→B: {(m_a-m_b)/m_a*100:.1f}%")


if __name__ == "__main__":
    main()
