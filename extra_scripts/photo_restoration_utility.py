"""
Historical Photo Restoration & Colorization Utility
--------------------------------------------------
This script provides local photo enhancement (contrast correction, color balancing,
denoising) and outputs formatted prompts/payloads for AI restoration with strict
facial identity preservation.

Usage:
    python photo_restoration_utility.py --input "path/to/photo.jpg"
    python photo_restoration_utility.py --folder "path/to/photos_folder"
"""

import argparse
from pathlib import Path

from PIL import Image, ImageEnhance, ImageFilter

MASTER_PROMPT = """Colorize and clean up the attached historical photograph with strict 100%
preservation of all subjects' exact faces, expressions, and identities.

STRICT CONSTRAINT: Do NOT alter, re-imagine, morph, or over-smooth any faces,
eyes, smiles, noses, lips, or expressions. Keep all facial features 100% faithful
to the original photograph.

Perform a conservative restoration:
- Remove dust, scratches, cracks, fading, scanner glare, and yellowing/sepia tinting.
- Preserve 100% of original facial contours, eye shapes, nose, lips, hair, and clothing silhouettes.
- Apply historically accurate, muted colorization with natural skin tones and era-appropriate clothing hues.
- Maintain original lighting, shadows, and natural film grain character.
"""


def enhance_local_photo(input_path: Path, output_path: Path | None = None) -> bool:
    """Applies high-quality local color balancing, contrast adjustment, and noise reduction."""
    if not input_path.exists():
        print(f"Error: Input file '{input_path}' does not exist.")
        return False

    if output_path is None:
        output_path = (
            input_path.parent / f"{input_path.stem} (Enhanced){input_path.suffix}"
        )

    print(f"Processing: {input_path.name} ...")

    img = Image.open(input_path)

    # 1. Convert to RGB if needed
    if img.mode != "RGB":
        img = img.convert("RGB")

    # 2. Slight noise cleanup (median filter for fine dust)
    cleaned = img.filter(ImageFilter.MedianFilter(size=3))

    # Blend median filter back with original to preserve sharp eyes/hair (80% original, 20% smooth)
    blended = Image.blend(img, cleaned, alpha=0.25)

    # 3. Enhance Contrast & Color Balance
    enhancer_contrast = ImageEnhance.Contrast(blended)
    enhanced = enhancer_contrast.enhance(1.15)

    enhancer_color = ImageEnhance.Color(enhanced)
    enhanced = enhancer_color.enhance(1.10)

    enhancer_sharp = ImageEnhance.Sharpness(enhanced)
    final_img = enhancer_sharp.enhance(1.10)

    final_img.save(output_path, quality=95)
    print(f"✅ Saved enhanced photo to: {output_path}")
    return True


def print_ai_prompt(image_path: Path) -> None:
    """Outputs the ready-to-use AI restoration prompt for a specific file."""
    print("\n" + "=" * 60)
    print(f"AI RESTORATION PROMPT FOR: {image_path.name}")
    print("=" * 60)
    print(MASTER_PROMPT)
    print("=" * 60 + "\n")


def main() -> None:
    """Process the requested images or print the restoration prompt."""
    parser = argparse.ArgumentParser(description="Historical Photo Restoration Utility")
    parser.add_argument("--input", "-i", type=str, help="Path to a single image file")
    parser.add_argument(
        "--folder", "-f", type=str, help="Path to a directory containing images"
    )
    parser.add_argument(
        "--prompt-only", action="store_true", help="Print the master restoration prompt"
    )

    args = parser.parse_args()

    if args.prompt_only or (not args.input and not args.folder):
        print(MASTER_PROMPT)
        return

    if args.input:
        img_path = Path(args.input)
        enhance_local_photo(img_path)
        print_ai_prompt(img_path)

    elif args.folder:
        folder_path = Path(args.folder)
        if not folder_path.exists():
            print(f"Error: Folder '{folder_path}' does not exist.")
            return

        valid_exts = {".jpg", ".jpeg", ".png", ".bmp", ".webp", ".tif", ".tiff"}
        images = [p for p in folder_path.iterdir() if p.suffix.lower() in valid_exts]

        print(f"Found {len(images)} images in '{folder_path.name}'...")
        for img in images:
            if "(Restored)" in img.name or "(Enhanced)" in img.name:
                continue
            enhance_local_photo(img)


if __name__ == "__main__":
    main()
