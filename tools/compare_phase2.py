#!/usr/bin/env python3
"""
Phase 2: Extract per-channel curves and test them.
Strategy: extract inverted→TIF curve (absorbing ALL processing) and measure residual.
Then factor out known steps to isolate the film curve.
"""
import numpy as np
import tifffile
import sys

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
    "test4": {
        "film_type": 0, "gamma": 2.0, "ev": 1.0, "saturation": 0,
        "shadow": [768, 768, 768, 768], "gray": [128, 128, 128, 128],
        "highlight": [16320, 16320, 16320, 16320],
        "dot_color": [0,0,0,0,0,0,0, 255,255,255,255,255,255,255],
    },
}


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
    """Extract 65536-point per-channel curve from src→tgt mapping."""
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
        
        # Fill gaps with linear interpolation (numpy only)
        valid_idx = np.where(mask)[0]
        if len(valid_idx) > 2:
            curve = np.interp(np.arange(BINS), valid_idx, curve[valid_idx])
        
        # Enforce monotonicity
        for i in range(1, BINS):
            if curve[i] < curve[i-1]:
                curve[i] = curve[i-1]
        
        curves.append(curve)
    return curves


def apply_curve(data, curves):
    """Apply per-channel curves via lookup."""
    result = np.empty_like(data)
    for ch in range(3):
        idx = data[:,:,ch].astype(np.int32).clip(0, 65535)
        result[:,:,ch] = curves[ch][idx]
    return result


def mae(a, b):
    return np.mean(np.abs(a - b))

def mae_per_ch(a, b):
    return [np.mean(np.abs(a[:,:,c] - b[:,:,c])) for c in range(3)]


def analyze_test2():
    """Focused analysis of test2 - simplest negative."""
    name = "test2"
    corr = CORRECTIONS[name]
    base = "/Users/will/vmwareShare/test_image"
    
    print(f"Loading {name}...")
    raw = tifffile.imread(f"{base}/{name}.fff", key=0).astype(np.float32)
    ref = tifffile.imread(f"{base}/{name}.tif", key=0).astype(np.float32)
    print(f"  raw: {raw.shape}, ref: {ref.shape}")
    
    inverted = step_invert(raw, corr)
    
    # ─── Test 1: Direct inverted→TIF curve ───
    print("\n=== Test 1: Direct inverted→TIF curve (absorbs everything) ===")
    curves_direct = extract_curve_65k(inverted, ref)
    applied_direct = apply_curve(inverted, curves_direct)
    m = mae(applied_direct, ref)
    mpc = mae_per_ch(applied_direct, ref)
    print(f"  MAE: {m:.2f}  R={mpc[0]:.2f} G={mpc[1]:.2f} B={mpc[2]:.2f}")
    print(f"  → This is the best we can do with per-channel curves (noise=USM+saturation)")
    
    # ─── Test 2: Pipeline ordering tests ───
    # Try: inv → CURVE → levels → gamma → sat  (curve before levels)
    print("\n=== Test 2: inv→CURVE→levels→gamma→sat ===")
    # To extract the curve: un-do sat, gamma, levels from TIF
    ref_work = ref.copy()
    
    # Un-saturation (BT.709)
    sat = corr["saturation"] / 100.0
    v = ref_work / 65535.0
    lum = 0.2126 * v[:,:,0] + 0.7152 * v[:,:,1] + 0.0722 * v[:,:,2]
    for ch in range(3):
        v[:,:,ch] = lum + (v[:,:,ch] - lum) / (1 + sat)
    ref_no_sat = v * 65535.0
    
    # Un-master-gamma
    gamma_m = max(corr["gamma"] - 1.0, 0.01)
    v = (ref_no_sat / 65535.0).clip(0, 1)
    v = np.power(v, gamma_m)
    ref_no_gam = v * 65535.0
    
    # Un-per-channel-gamma
    for ch in range(3):
        gamma_c = max(corr["gray"][ch + 1] / 128.0, 0.01)
        v[:,:,ch] = np.power((ref_no_gam[:,:,ch] / 65535.0).clip(0, 1), gamma_c)
    ref_no_chgam = v * 65535.0
    
    # Un-levels
    for ch in range(3):
        bl = corr["shadow"][ch + 1] * 4.0 / 65535.0
        wh = corr["highlight"][ch + 1] * 4.0 / 65535.0
        rng = max(wh - bl, 0.001)
        v[:,:,ch] = ((ref_no_chgam[:,:,ch] / 65535.0) * rng + bl).clip(0, 1)
    curve_target = v * 65535.0
    
    # Now extract curve: inverted → curve_target
    curves_pre = extract_curve_65k(inverted, curve_target)
    
    # Apply: inv → curve → levels → gamma → sat → compare with ref
    applied = apply_curve(inverted, curves_pre)
    applied = step_levels(applied, corr)
    applied = step_per_ch_gamma(applied, corr)
    applied = step_master_gamma(applied, corr)
    applied = step_saturation(applied, corr)
    m = mae(applied, ref)
    mpc = mae_per_ch(applied, ref)
    print(f"  MAE: {m:.2f}  R={mpc[0]:.2f} G={mpc[1]:.2f} B={mpc[2]:.2f}")
    
    # ─── Test 3: Try inv → levels → gamma → CURVE → sat ───
    print("\n=== Test 3: inv→levels→gamma→CURVE→sat ===")
    data_lg = step_levels(inverted, corr)
    data_lg = step_per_ch_gamma(data_lg, corr)
    data_lg = step_master_gamma(data_lg, corr)
    
    # Curve target = un-sat(TIF)
    curves_post = extract_curve_65k(data_lg, ref_no_sat)
    applied = apply_curve(data_lg, curves_post)
    applied = step_saturation(applied, corr)
    m = mae(applied, ref)
    mpc = mae_per_ch(applied, ref)
    print(f"  MAE: {m:.2f}  R={mpc[0]:.2f} G={mpc[1]:.2f} B={mpc[2]:.2f}")
    
    # ─── Test 4: Try inv → CURVE → sat → levels → gamma ───
    print("\n=== Test 4: inv→CURVE→sat→levels→gamma ===")
    # Target for curve = un_gamma(un_levels(un_sat_from_before_levels(TIF)))
    # This is getting complex. Let me reverse differently.
    # TIF = gamma(levels(sat(CURVE(inv))))
    # So: un_gamma(un_levels(TIF)) = sat(CURVE(inv))
    # un_sat(un_gamma(un_levels(TIF))) = CURVE(inv)
    ref_work2 = ref.copy()
    # Un-master-gamma
    v = np.power((ref_work2 / 65535.0).clip(0, 1), gamma_m)
    # Un-per-channel gamma
    for ch in range(3):
        gamma_c = max(corr["gray"][ch + 1] / 128.0, 0.01)
        v[:,:,ch] = np.power(v[:,:,ch].clip(0, 1), gamma_c)
    # Un-levels
    for ch in range(3):
        bl = corr["shadow"][ch + 1] * 4.0 / 65535.0
        wh = corr["highlight"][ch + 1] * 4.0 / 65535.0
        rng = max(wh - bl, 0.001)
        v[:,:,ch] = (v[:,:,ch] * rng + bl).clip(0, 1)
    ref_ulg = v * 65535.0
    # Un-sat
    v = ref_ulg / 65535.0
    lum = 0.2126 * v[:,:,0] + 0.7152 * v[:,:,1] + 0.0722 * v[:,:,2]
    for ch in range(3):
        v[:,:,ch] = lum + (v[:,:,ch] - lum) / (1 + sat)
    curve_target_4 = (v * 65535.0).clip(0, 65535)
    
    curves_4 = extract_curve_65k(inverted, curve_target_4)
    applied = apply_curve(inverted, curves_4)
    applied = step_saturation(applied, corr)
    applied = step_levels(applied, corr)
    applied = step_per_ch_gamma(applied, corr)
    applied = step_master_gamma(applied, corr)
    m = mae(applied, ref)
    mpc = mae_per_ch(applied, ref)
    print(f"  MAE: {m:.2f}  R={mpc[0]:.2f} G={mpc[1]:.2f} B={mpc[2]:.2f}")
    
    # ─── Test 5: Try BT.601 weights for saturation ───
    print("\n=== Test 5: inv→CURVE→levels→gamma→sat(BT.601) ===")
    # Re-extract with BT.601 un-saturation
    v = ref.copy() / 65535.0
    lum601 = 0.299 * v[:,:,0] + 0.587 * v[:,:,1] + 0.114 * v[:,:,2]
    for ch in range(3):
        v[:,:,ch] = lum601 + (v[:,:,ch] - lum601) / (1 + sat)
    ref_no_sat601 = v * 65535.0
    
    # Un-gamma, un-levels
    v = np.power((ref_no_sat601 / 65535.0).clip(0, 1), gamma_m)
    for ch in range(3):
        gamma_c = max(corr["gray"][ch + 1] / 128.0, 0.01)
        v[:,:,ch] = np.power(v[:,:,ch].clip(0, 1), gamma_c)
    for ch in range(3):
        bl = corr["shadow"][ch + 1] * 4.0 / 65535.0
        wh = corr["highlight"][ch + 1] * 4.0 / 65535.0
        rng = max(wh - bl, 0.001)
        v[:,:,ch] = (v[:,:,ch] * rng + bl).clip(0, 1)
    curve_target_601 = (v * 65535.0).clip(0, 65535)
    
    curves_601 = extract_curve_65k(inverted, curve_target_601)
    applied = apply_curve(inverted, curves_601)
    applied = step_levels(applied, corr)
    applied = step_per_ch_gamma(applied, corr)
    applied = step_master_gamma(applied, corr)
    applied = step_saturation(applied, corr, weights="601")
    m = mae(applied, ref)
    mpc = mae_per_ch(applied, ref)
    print(f"  MAE: {m:.2f}  R={mpc[0]:.2f} G={mpc[1]:.2f} B={mpc[2]:.2f}")
    
    # ─── Test 6: No saturation at all ───
    print("\n=== Test 6: inv→CURVE→levels→gamma (NO saturation) ===")
    # What if FlexColor applies saturation differently or as part of the curve?
    v = np.power((ref / 65535.0).clip(0, 1), gamma_m)
    for ch in range(3):
        gamma_c = max(corr["gray"][ch + 1] / 128.0, 0.01)
        v[:,:,ch] = np.power(v[:,:,ch].clip(0, 1), gamma_c)
    for ch in range(3):
        bl = corr["shadow"][ch + 1] * 4.0 / 65535.0
        wh = corr["highlight"][ch + 1] * 4.0 / 65535.0
        rng = max(wh - bl, 0.001)
        v[:,:,ch] = (v[:,:,ch] * rng + bl).clip(0, 1)
    curve_target_nosat = (v * 65535.0).clip(0, 65535)
    
    curves_nosat = extract_curve_65k(inverted, curve_target_nosat)
    applied = apply_curve(inverted, curves_nosat)
    applied = step_levels(applied, corr)
    applied = step_per_ch_gamma(applied, corr)
    applied = step_master_gamma(applied, corr)
    m_nosat = mae(applied, ref)
    mpc_nosat = mae_per_ch(applied, ref)
    print(f"  MAE (no sat output): {m_nosat:.2f}  R={mpc_nosat[0]:.2f} G={mpc_nosat[1]:.2f} B={mpc_nosat[2]:.2f}")
    # Now add saturation
    applied_sat = step_saturation(applied, corr)
    m_sat = mae(applied_sat, ref)
    mpc_sat = mae_per_ch(applied_sat, ref)
    print(f"  MAE (with sat output): {m_sat:.2f}  R={mpc_sat[0]:.2f} G={mpc_sat[1]:.2f} B={mpc_sat[2]:.2f}")
    
    # ─── Test 7: Cross-validate with test3 ───
    print("\n=== Test 7: Apply test2 curve to test3 data ===")
    corr3 = CORRECTIONS["test3"]
    raw3 = tifffile.imread(f"{base}/test3.fff", key=0).astype(np.float32)
    ref3 = tifffile.imread(f"{base}/test3.tif", key=0).astype(np.float32)
    inv3 = step_invert(raw3, corr3)
    
    # Apply test2's curve to test3 data
    applied3 = apply_curve(inv3, curves_pre)  # curves_pre = test2's pre-levels curve
    applied3 = step_levels(applied3, corr3)
    applied3 = step_per_ch_gamma(applied3, corr3)
    applied3 = step_master_gamma(applied3, corr3)
    applied3 = step_saturation(applied3, corr3)
    m3 = mae(applied3, ref3)
    mpc3 = mae_per_ch(applied3, ref3)
    print(f"  MAE: {m3:.2f}  R={mpc3[0]:.2f} G={mpc3[1]:.2f} B={mpc3[2]:.2f}")
    print(f"  (If low, curve is transferable between same-film images)")
    
    # Also extract test3's own curve and compare
    print("\n=== Test 8: Test3's own extracted curve ===")
    v3 = ref3.copy() / 65535.0
    sat3 = corr3["saturation"] / 100.0
    lum3 = 0.2126 * v3[:,:,0] + 0.7152 * v3[:,:,1] + 0.0722 * v3[:,:,2]
    for ch in range(3):
        v3[:,:,ch] = lum3 + (v3[:,:,ch] - lum3) / (1 + sat3)
    v3 = np.power(v3.clip(0, 1), gamma_m)
    for ch in range(3):
        gamma_c = max(corr3["gray"][ch + 1] / 128.0, 0.01)
        v3[:,:,ch] = np.power(v3[:,:,ch].clip(0, 1), gamma_c)
    for ch in range(3):
        bl = corr3["shadow"][ch + 1] * 4.0 / 65535.0
        wh = corr3["highlight"][ch + 1] * 4.0 / 65535.0
        rng = max(wh - bl, 0.001)
        v3[:,:,ch] = (v3[:,:,ch] * rng + bl).clip(0, 1)
    ct3 = (v3 * 65535.0).clip(0, 65535)
    
    curves3 = extract_curve_65k(inv3, ct3)
    applied3b = apply_curve(inv3, curves3)
    applied3b = step_levels(applied3b, corr3)
    applied3b = step_per_ch_gamma(applied3b, corr3)
    applied3b = step_master_gamma(applied3b, corr3)
    applied3b = step_saturation(applied3b, corr3)
    m3b = mae(applied3b, ref3)
    mpc3b = mae_per_ch(applied3b, ref3)
    print(f"  MAE: {m3b:.2f}  R={mpc3b[0]:.2f} G={mpc3b[1]:.2f} B={mpc3b[2]:.2f}")
    
    # Compare curves between test2 and test3
    print("\n=== Curve comparison: test2 vs test3 ===")
    for ch, name in enumerate(["R", "G", "B"]):
        diff = np.abs(np.array(curves_pre[ch]) - np.array(curves3[ch]))
        print(f"  {name}: MAE={np.mean(diff):.2f}, max_diff={np.max(diff):.2f}")
        # Sample points
        for i in [0, 8192, 16384, 24576, 32768, 40960, 49152, 57344, 65535]:
            print(f"    [{i:5d}] test2={curves_pre[ch][i]:8.1f} test3={curves3[ch][i]:8.1f} Δ={curves_pre[ch][i]-curves3[ch][i]:+8.1f}")


if __name__ == "__main__":
    base = "/Users/will/vmwareShare/test_image"
    analyze_test2()
