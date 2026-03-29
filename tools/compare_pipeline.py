#!/usr/bin/env python3
"""
Compare FFF raw 16-bit data with FlexColor-exported 16-bit TIF.
Step-by-step pipeline comparison to reverse-engineer the exact FlexColor processing.
"""
import sys
import numpy as np
import tifffile

# ─── Test file correction parameters ─────────────────────────────────────
CORRECTIONS = {
    "test1": {
        "film_type": 1,  # Negative C-41
        "gamma": 2.0, "ev": 1.0, "saturation": 15,
        "shadow": [448, 448, 4352, 8832],
        "gray": [128, 134, 131, 108],
        "highlight": [15680, 11520, 14720, 15680],
        "dot_color": [0,0,0,0,0,0,0, 255,255,255,255,255,255,255],
        "input_profile": "Flextight Input",
        "rgb_profile": "sRGB Color Space Profile.icm",
    },
    "test2": {
        "film_type": 1,
        "gamma": 2.0, "ev": 1.0, "saturation": 25,
        "shadow": [0, 0, 0, 0],
        "gray": [128, 94, 92, 90],
        "highlight": [16192, 16192, 16192, 16192],
        "dot_color": [0,0,0,0,0,0,0, 255,255,255,255,255,255,255],
        "input_profile": "",
        "rgb_profile": "",
    },
    "test3": {
        "film_type": 1,
        "gamma": 2.0, "ev": 1.0, "saturation": 25,
        "shadow": [2496, 2496, 5376, 8640],
        "gray": [128, 94, 92, 90],
        "highlight": [16192, 16192, 16192, 16192],
        "dot_color": [0,0,0,0,0,0,0, 255,255,255,255,255,255,255],
        "input_profile": "",
        "rgb_profile": "",
    },
    "test4": {
        "film_type": 0,  # Positive E-6
        "gamma": 2.0, "ev": 1.0, "saturation": 0,
        "shadow": [768, 768, 768, 768],
        "gray": [128, 128, 128, 128],
        "highlight": [16320, 16320, 16320, 16320],
        "dot_color": [0,0,0,0,0,0,0, 255,255,255,255,255,255,255],
        "input_profile": "Flextight X5 & 949",
        "rgb_profile": "sRGB Color Space Profile.icm",
    },
}


def load_fff_raw(path):
    """Load full-resolution 16-bit raw from FFF file (IFD#0)."""
    tif = tifffile.TiffFile(path)
    raw = tif.pages[0].asarray()
    print(f"  FFF raw: {raw.shape}, dtype={raw.dtype}")
    return raw.astype(np.float32)


def load_tif_ref(path):
    """Load FlexColor-exported 16-bit TIF as reference."""
    tif = tifffile.TiffFile(path)
    ref = tif.pages[0].asarray()
    print(f"  TIF ref: {ref.shape}, dtype={ref.dtype}")
    return ref.astype(np.float32)


def mae(a, b):
    return np.mean(np.abs(a - b))

def mae_per_ch(a, b):
    return [np.mean(np.abs(a[:,:,c] - b[:,:,c])) for c in range(3)]

def percentile_error(a, b, pcts=[50, 90, 95, 99]):
    err = np.abs(a - b).flatten()
    return {p: np.percentile(err, p) for p in pcts}


def report(name, result, ref):
    m = mae(result, ref)
    mpc = mae_per_ch(result, ref)
    pcts = percentile_error(result, ref)
    print(f"  [{name}]")
    print(f"    MAE total: {m:.2f}  R={mpc[0]:.2f} G={mpc[1]:.2f} B={mpc[2]:.2f}")
    print(f"    p50={pcts[50]:.1f} p90={pcts[90]:.1f} p95={pcts[95]:.1f} p99={pcts[99]:.1f}")
    return m


# ─── Pipeline steps ──────────────────────────────────────────────────────

def step_invert(raw, corr):
    result = np.copy(raw)
    for ch in range(3):
        hi = corr["highlight"][ch + 1] * 4.0
        if hi > 0:
            inv = (hi - raw[:,:,ch]).clip(0) * (65535.0 / hi)
        else:
            inv = raw[:,:,ch]
        result[:,:,ch] = inv.clip(0, 65535)
    return result


def step_levels(data, corr):
    result = np.empty_like(data)
    for ch in range(3):
        bl = corr["shadow"][ch + 1] * 4.0 / 65535.0
        wh = corr["highlight"][ch + 1] * 4.0 / 65535.0
        rng = max(wh - bl, 0.001)
        v = data[:,:,ch] / 65535.0
        v = ((v - bl) / rng).clip(0, 1)
        result[:,:,ch] = v * 65535.0
    return result


def step_per_ch_gamma(data, corr):
    result = np.empty_like(data)
    for ch in range(3):
        gamma_c = max(corr["gray"][ch + 1] / 128.0, 0.01)
        v = (data[:,:,ch] / 65535.0).clip(0, 1)
        v = np.power(v, 1.0 / gamma_c)
        result[:,:,ch] = v * 65535.0
    return result


def step_master_gamma(data, corr):
    gamma_m = max(corr["gamma"] - 1.0, 0.01)
    v = (data / 65535.0).clip(0, 1)
    v = np.power(v, 1.0 / gamma_m)
    return v * 65535.0


def step_output_levels(data, corr):
    dc = corr["dot_color"]
    out_lo = dc[0] / 255.0
    out_hi = dc[7] / 255.0
    out_range = max(out_hi - out_lo, 0.001)
    v = data / 65535.0
    v = out_lo + v * out_range
    return (v * 65535.0).clip(0, 65535)


def step_exposure(data, corr):
    ev = corr["ev"]
    if abs(ev - 1.0) < 0.001:
        return data
    mult = 2.0 ** (ev - 1.0)
    return (data * mult).clip(0, 65535)


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


# ─── Pre-levels film curve extraction (16-bit to 16-bit) ─────────────────

def extract_pre_levels_curve(raw, ref, corr):
    """
    If film_curve is applied BEFORE levels:
      raw → invert → FILM_CURVE → levels → gamma → out → exp → sat = ref
    
    So: FILM_CURVE(inverted) = un_sat(un_exp(un_out(un_gamma(un_levels(ref)))))
    """
    is_neg = corr["film_type"] in (1, 2)
    if not is_neg:
        print("  Skipping pre-levels curve extraction (positive film)")
        return
    
    print("\n  === Pre-levels film curve extraction (16-bit) ===")
    
    # 1. Invert the raw data
    inverted = step_invert(raw, corr)
    
    # 2. Reverse-process the TIF
    target = ref.copy() / 65535.0
    
    # Un-saturation
    sat = corr["saturation"] / 100.0
    if abs(sat) > 0.001:
        lum = 0.2126 * target[:,:,0] + 0.7152 * target[:,:,1] + 0.0722 * target[:,:,2]
        for ch in range(3):
            target[:,:,ch] = lum + (target[:,:,ch] - lum) / (1 + sat)
    
    # Un-exposure
    ev = corr["ev"]
    if abs(ev - 1.0) > 0.001:
        target /= 2.0 ** (ev - 1.0)
    
    # Un-output-levels
    dc = corr["dot_color"]
    out_lo = dc[0] / 255.0
    out_hi = dc[7] / 255.0
    out_range = max(out_hi - out_lo, 0.001)
    target = ((target - out_lo) / out_range).clip(0, 1)
    
    # Un-master-gamma
    gamma_m = max(corr["gamma"] - 1.0, 0.01)
    target = np.power(target.clip(0, 1), gamma_m)
    
    # Un-per-channel-gamma
    for ch in range(3):
        gamma_c = max(corr["gray"][ch + 1] / 128.0, 0.01)
        target[:,:,ch] = np.power(target[:,:,ch].clip(0, 1), gamma_c)
    
    # Un-levels
    for ch in range(3):
        bl = corr["shadow"][ch + 1] * 4.0 / 65535.0
        wh = corr["highlight"][ch + 1] * 4.0 / 65535.0
        rng = max(wh - bl, 0.001)
        target[:,:,ch] = (target[:,:,ch] * rng + bl).clip(0, 1)
    
    target_16 = (target * 65535.0).clip(0, 65535)
    
    # Build binned curve: inverted → target_16
    BINS = 1024
    for ch_name, ch in [("R", 0), ("G", 1), ("B", 2)]:
        inv = inverted[:,:,ch].flatten()
        tgt = target_16[:,:,ch].flatten()
        
        bins = np.zeros(BINS, dtype=np.float64)
        counts = np.zeros(BINS, dtype=np.int64)
        
        bin_idx = (inv * (BINS - 1) / 65535).astype(np.int32).clip(0, BINS - 1)
        np.add.at(bins, bin_idx, tgt.astype(np.float64))
        np.add.at(counts, bin_idx, 1)
        
        mask = counts > 0
        bins[mask] /= counts[mask]
        
        # Check monotonicity
        valid_bins = bins[mask]
        diffs = np.diff(valid_bins)
        n_non_mono = np.sum(diffs < -10)  # allow small noise
        
        # Identity comparison
        identity = np.arange(BINS) * 65535.0 / (BINS - 1)
        identity_mae = np.mean(np.abs(bins[mask] - identity[mask]))
        
        print(f"\n  {ch_name} film curve: non-mono={n_non_mono}, MAE-vs-identity={identity_mae:.2f}")
        print(f"    Samples:")
        for i in [0, 32, 64, 128, 192, 256, 384, 512, 640, 768, 896, 1023]:
            if i < BINS and counts[i] > 0:
                identity_val = i * 65535.0 / (BINS - 1)
                ratio = bins[i] / max(identity_val, 1)
                print(f"      [{i:4d}] inv={identity_val:8.1f} → curve_out={bins[i]:8.1f} (ratio={ratio:.4f}, n={counts[i]})")


def extract_post_invert_curve(raw, ref, corr):
    """
    Alternative: what if the ENTIRE processing (levels+gamma+sat+...) is
    absorbed into one big per-channel curve? Just map inverted → TIF directly.
    """
    is_neg = corr["film_type"] in (1, 2)
    if not is_neg:
        return
    
    print("\n  === Direct inverted→TIF curve (all processing absorbed) ===")
    
    inverted = step_invert(raw, corr)
    
    BINS = 1024
    for ch_name, ch in [("R", 0), ("G", 1), ("B", 2)]:
        inv = inverted[:,:,ch].flatten()
        tgt = ref[:,:,ch].flatten()
        
        bins = np.zeros(BINS, dtype=np.float64)
        counts = np.zeros(BINS, dtype=np.int64)
        
        bin_idx = (inv * (BINS - 1) / 65535).astype(np.int32).clip(0, BINS - 1)
        np.add.at(bins, bin_idx, tgt.astype(np.float64))
        np.add.at(counts, bin_idx, 1)
        
        mask = counts > 0
        bins[mask] /= counts[mask]
        
        # Variance per bin (to check if it's a clean function)
        var_bins = np.zeros(BINS, dtype=np.float64)
        np.add.at(var_bins, bin_idx, (tgt.astype(np.float64) - bins[bin_idx])**2)
        var_bins[mask] /= counts[mask]
        avg_std = np.mean(np.sqrt(var_bins[mask]))
        
        diffs = np.diff(bins[mask])
        n_non_mono = np.sum(diffs < -10)
        
        print(f"\n  {ch_name}: non-mono={n_non_mono}, avg_std_per_bin={avg_std:.2f}")
        print(f"    Curve samples:")
        for i in [0, 32, 64, 128, 192, 256, 384, 512, 640, 768, 896, 1023]:
            if i < BINS and counts[i] > 0:
                identity_val = i * 65535.0 / (BINS - 1)
                std = np.sqrt(var_bins[i]) if counts[i] > 0 else 0
                print(f"      [{i:4d}] inv={identity_val:8.1f} → TIF={bins[i]:8.1f} (std={std:.1f}, n={counts[i]})")


# ─── Main ────────────────────────────────────────────────────────────────

def analyze_file(name, fff_path, tif_path):
    corr = CORRECTIONS[name]
    print(f"\n{'='*70}")
    print(f"  {name}: FilmType={corr['film_type']} γ={corr['gamma']} Sat={corr['saturation']}")
    print(f"  Shadow={corr['shadow']} Gray={corr['gray']} Highlight={corr['highlight']}")
    print(f"  ICC: {corr['input_profile']} → {corr['rgb_profile']}")
    print(f"{'='*70}")
    
    raw = load_fff_raw(fff_path)
    ref = load_tif_ref(tif_path)
    
    if raw.shape != ref.shape:
        print(f"  ERROR: shape mismatch! raw={raw.shape} ref={ref.shape}")
        return
    
    # Step-by-step pipeline comparison
    print(f"\n  --- Pipeline step-by-step (no film curve) ---")
    is_neg = corr["film_type"] in (1, 2)
    data = raw.copy()
    
    if is_neg:
        data = step_invert(data, corr)
        report("1_invert", data, ref)
    
    data_lev = step_levels(data, corr)
    report("2_levels", data_lev, ref)
    
    data_gam = step_per_ch_gamma(data_lev, corr)
    report("3_per_ch_gamma", data_gam, ref)
    
    data_mgam = step_master_gamma(data_gam, corr)
    report("4_master_gamma", data_mgam, ref)
    
    data_out = step_output_levels(data_mgam, corr)
    report("5_output_levels", data_out, ref)
    
    data_exp = step_exposure(data_out, corr)
    report("6_exposure", data_exp, ref)
    
    data_sat = step_saturation(data_exp, corr)
    report("7_saturation", data_sat, ref)
    
    # Extract curves
    extract_post_invert_curve(raw, ref, corr)
    extract_pre_levels_curve(raw, ref, corr)


if __name__ == "__main__":
    base = "/Users/will/vmwareShare/test_image"
    files = sys.argv[1:] if len(sys.argv) > 1 else ["test2"]
    for name in files:
        if name in CORRECTIONS:
            analyze_file(name, f"{base}/{name}.fff", f"{base}/{name}.tif")
        else:
            print(f"Unknown test file: {name}")
