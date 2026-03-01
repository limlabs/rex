import type { RexApiRequest, RexApiResponse } from "rex/server";

export default function handler(req: RexApiRequest, res: RexApiResponse) {
  res.status(200).json({ message: "Hello from Rex API!", method: req.method });
}
