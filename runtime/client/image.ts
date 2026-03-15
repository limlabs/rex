// Client-side Image component for rex/image
// Same rendering as server, plus onLoad clears blur placeholder.
import React from "react";

// Next.js static imports return { src, width, height, blurDataURL }
interface StaticImport {
  src: string;
  width?: number;
  height?: number;
  blurDataURL?: string;
}

interface ImageProps {
  src: string | StaticImport;
  width?: number;
  height?: number;
  alt?: string;
  quality?: number;
  priority?: boolean;
  placeholder?: string;
  blurDataURL?: string;
  fill?: boolean;
  sizes?: string;
  objectFit?: string;
  className?: string;
  id?: string;
  onLoad?: (e: React.SyntheticEvent<HTMLImageElement>) => void;
}

// Device-scale widths matching next/image breakpoints
const DEVICE_SIZES = [640, 750, 828, 1080, 1200, 1920];
const ICON_SIZES = [16, 32, 48, 64, 96, 128, 256, 384];

function buildSrc(src: string, w: number, q: number): string {
  return (
    "/_rex/image?url=" +
    encodeURIComponent(src) +
    "&w=" +
    w +
    "&q=" +
    q
  );
}

function buildSrcSet(src: string, width: number, quality: number): string {
  const all = ICON_SIZES.concat(DEVICE_SIZES);
  const limit = width * 2;
  const parts: string[] = [];
  for (let i = 0; i < all.length; i++) {
    const w = all[i];
    if (w < Math.floor(width / 4)) continue;
    if (w <= limit) {
      parts.push(buildSrc(src, w, quality) + " " + w + "w");
    }
  }
  if (parts.length === 0) {
    parts.push(buildSrc(src, width, quality) + " " + width + "w");
  }
  return parts.join(", ");
}

export default function Image(props: ImageProps): React.ReactElement {
  // Handle static imports: { src, width, height, blurDataURL }
  const rawSrc = props.src;
  let src: string;
  let staticWidth: number | undefined;
  let staticHeight: number | undefined;
  let staticBlur: string | undefined;
  if (typeof rawSrc === "object" && rawSrc !== null && typeof (rawSrc as any).src === "string") {
    src = (rawSrc as any).src;
    staticWidth = (rawSrc as any).width;
    staticHeight = (rawSrc as any).height;
    staticBlur = (rawSrc as any).blurDataURL;
  } else {
    src = rawSrc as string;
  }
  const alt = props.alt;
  const width = props.width ?? staticWidth;
  const height = props.height ?? staticHeight;
  const quality = props.quality ?? 75;
  const priority = props.priority ?? false;
  const placeholder = props.placeholder;
  const blurDataURL = props.blurDataURL ?? staticBlur;
  const fill = props.fill ?? false;
  const sizes = props.sizes;

  // Base style: block display, prevent overflow, maintain aspect ratio (like next/image)
  let style: Record<string, string | number> = {
    display: "block",
    maxWidth: "100%",
    height: "auto",
  };

  const imgProps: Record<string, unknown> = {
    alt: alt || "",
    loading: priority ? "eager" : "lazy",
    decoding: "async",
  };

  if (fill) {
    style = {
      position: "absolute",
      top: 0,
      left: 0,
      width: "100%",
      height: "100%",
      objectFit: props.objectFit || "cover",
    };
  } else {
    imgProps.width = width;
    imgProps.height = height;
  }

  if (priority) {
    imgProps.fetchPriority = "high";
  }

  // Set src and srcSet — SVGs are vector, skip the raster image optimizer
  const isSvg = src.endsWith(".svg");
  if (isSvg) {
    imgProps.src = src;
  } else {
    imgProps.src = buildSrc(src, width || 1920, quality);
    if (width) {
      imgProps.srcSet = buildSrcSet(src, width, quality);
    }
  }

  if (sizes) {
    imgProps.sizes = sizes;
  } else if (width) {
    imgProps.sizes = "(max-width: " + width + "px) 100vw, " + width + "px";
  }

  // Blur placeholder as background
  const hasBlur = placeholder === "blur" && blurDataURL;
  if (hasBlur) {
    style.backgroundImage = "url(" + blurDataURL + ")";
    style.backgroundSize = "cover";
    style.backgroundRepeat = "no-repeat";
  }

  imgProps.style = style;

  // Clear blur placeholder on load
  const userOnLoad = props.onLoad;
  if (hasBlur || userOnLoad) {
    imgProps.onLoad = function (e: React.SyntheticEvent<HTMLImageElement>) {
      if (hasBlur) {
        const target = e.target as HTMLImageElement;
        if (target?.style) {
          target.style.backgroundImage = "";
        }
      }
      if (userOnLoad) userOnLoad(e);
    };
  }

  if (props.className) imgProps.className = props.className;
  if (props.id) imgProps.id = props.id;

  return React.createElement("img", imgProps);
}
