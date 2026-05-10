#!/usr/bin/env python3
"""Generate team-color mask PNGs from red-dominant pixels in sprites."""

from __future__ import annotations

import argparse
from pathlib import Path

from PIL import Image


DEFAULT_SPRITES = ("archer", "cavalry", "knight", "settler", "warrior")


def is_mask_pixel(
    red: int,
    green: int,
    blue: int,
    *,
    min_red: int,
    min_margin: int,
    min_ratio: float,
) -> bool:
    return (
        red >= min_red
        and red - green >= min_margin
        and red - blue >= min_margin
        and red >= green * min_ratio
        and red >= blue * min_ratio
    )


def generate_mask(
    source: Path,
    output: Path,
    *,
    min_red: int,
    min_margin: int,
    min_ratio: float,
) -> int:
    image = Image.open(source).convert("RGBA")
    mask = Image.new("RGBA", image.size, (255, 255, 255, 0))

    source_pixels = image.load()
    mask_pixels = mask.load()
    selected = 0

    for y in range(image.height):
        for x in range(image.width):
            red, green, blue, alpha = source_pixels[x, y]
            if alpha == 0:
                continue

            if is_mask_pixel(
                red,
                green,
                blue,
                min_red=min_red,
                min_margin=min_margin,
                min_ratio=min_ratio,
            ):
                mask_pixels[x, y] = (255, 255, 255, alpha)
                selected += 1

    output.parent.mkdir(parents=True, exist_ok=True)
    mask.save(output)
    return selected


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Create *_team_mask.png files from red-dominant sprite pixels."
    )
    parser.add_argument(
        "--input-dir",
        type=Path,
        default=Path("assets/textures/units"),
        help="Directory containing sprite PNGs.",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=None,
        help="Directory for generated masks. Defaults to --input-dir.",
    )
    parser.add_argument(
        "--units",
        nargs="+",
        dest="sprites",
        help="Deprecated alias for --sprites.",
    )
    parser.add_argument(
        "--sprites",
        nargs="+",
        default=None,
        help="Sprite base names to convert, without .png.",
    )
    parser.add_argument(
        "--suffix",
        default="_team_mask",
        help="Suffix appended to each unit name for output files.",
    )
    parser.add_argument(
        "--min-red",
        type=int,
        default=50,
        help="Minimum red channel value required for a pixel to be selected.",
    )
    parser.add_argument(
        "--min-margin",
        type=int,
        default=70,
        help="Minimum amount red must exceed green and blue.",
    )
    parser.add_argument(
        "--min-ratio",
        type=float,
        default=3.0,
        help="Minimum ratio red must have over green and blue.",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    output_dir = args.output_dir or args.input_dir
    sprites = args.sprites or list(DEFAULT_SPRITES)

    for sprite in sprites:
        source = args.input_dir / f"{sprite}.png"
        output = output_dir / f"{sprite}{args.suffix}.png"

        if not source.exists():
            raise FileNotFoundError(f"Missing source sprite: {source}")

        selected = generate_mask(
            source,
            output,
            min_red=args.min_red,
            min_margin=args.min_margin,
            min_ratio=args.min_ratio,
        )
        print(f"{sprite}: {selected} mask pixels -> {output}")


if __name__ == "__main__":
    main()
