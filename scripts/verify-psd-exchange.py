"""Optional independent fixture check; psd-tools/Pillow/NumPy are verification tools only."""
import argparse
from pathlib import Path

import numpy as np
from psd_tools import PSDImage


def verify(path):
    psd = PSDImage.open(path)
    header = psd._record.header
    assert header.depth == 8 and header.channels == 4, path.name
    planes = psd._record.image_data.get_data(header)
    expected = np.stack(
        [np.frombuffer(p, dtype=np.uint8).reshape(psd.height, psd.width) for p in planes],
        axis=-1,
    ).astype(float)
    assert psd._record.layer_and_mask_information.layer_info.layer_count < 0
    # The stored merged RGB is white-matted; alpha stays independent.
    alpha = expected[:, :, 3:4]
    expected[:, :, :3] = np.divide(
        expected[:, :, :3] * 255 - 255 * (255 - alpha),
        alpha,
        out=np.zeros_like(expected[:, :, :3]),
        where=alpha > 0,
    ).clip(0, 255)
    actual = np.asarray(
        psd.composite(force=True, ignore_preview=True, apply_icc=False).convert("RGBA")
    ).astype(float)
    assert actual.shape == expected.shape, path.name
    for values in (actual, expected):
        values[:, :, :3] *= values[:, :, 3:4] / 255
    delta = float(np.max(np.abs(actual - expected)))
    # Independent 8-bit compositing and white-matte quantization can differ by two levels.
    assert delta <= 2.01, (path.name, delta)
    if path.stem.startswith("layers-"):
        assert [layer.name for layer in psd] == ["Base", "그룹 ✏", "Hidden"]
        assert psd[1].is_group() and len(psd[1]) == 1 and psd[1].mask is not None
        assert psd[1][0].name == "Alpha" and psd[1][0].opacity == 166
        assert not psd[2].visible
    if path.stem in ("mode-SoftLight", "mode-GlowDodge", "mode-AddGlow"):
        assert len(psd) == 1, "Unverified blend must be baked with its backdrop"
    print(f"{path.name}: roots={len(psd)} premultiplied_delta={delta:.3f}")
    return delta


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fixtures", type=Path)
    args = parser.parse_args()
    files = sorted(
        p for p in args.fixtures.iterdir()
        if p.suffix.lower() in (".psd", ".psb") and not p.name.startswith("unsupported-")
    )
    assert files, "No exported PSD/PSB fixtures"
    maximum = max(verify(p) for p in files)
    print(f"Verified {len(files)} independent PSD/PSB composites; max delta={maximum:.3f}")


if __name__ == "__main__":
    main()
