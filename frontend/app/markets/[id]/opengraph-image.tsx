import { ImageResponse } from "next/server";

import { buildPoolTitle, sanitizeText } from "../../../lib/metadata";
import { getPoolById, isValidPoolId } from "../../../lib/pools";

export const alt = "Market preview image";
export const size = { width: 1200, height: 630 } as const;
export const contentType = "image/png";

export default async function OpenGraphImage({
  params,
}: {
  params: { id: string };
}) {
  const id = typeof params.id === "string" ? params.id : "";
  const pool = isValidPoolId(id) ? await getPoolById(id) : null;
  const title = buildPoolTitle(pool);
  const subtitle = pool?.description
    ? sanitizeText(pool.description, 140)
    : "Market preview";

  return new ImageResponse(
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        justifyContent: "space-between",
        width: "100%",
        height: "100%",
        padding: "64px",
        background: "linear-gradient(135deg, #0f172a 0%, #1e293b 100%)",
        color: "#ffffff",
        fontFamily: "sans-serif",
      }}
    >
      <div style={{ display: "flex", flexDirection: "column", gap: "16px" }}>
        <div style={{ fontSize: 28, color: "#94a3b8" }}>Markets</div>
        <div style={{ fontSize: 64, fontWeight: 700, lineHeight: 1.1 }}>
          {title}
        </div>
        <div style={{ fontSize: 28, color: "#cbd5e1" }}>{subtitle}</div>
      </div>
      <div style={{ display: "flex", justifyContent: "space-between", fontSize: 24 }}>
        <span>{pool ? `TVL: ${pool.tvL.toLocaleString()}` : "Unavailable"}</span>
        <span>{pool?.apr != null ? `APR: ${pool.apr * 100}%` : ""}</span>
      </div>
    </div>,
    { width: size.width, height: size.height },
  );
}
