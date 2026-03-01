// Simple API route to verify pages/ router works alongside app/ router
export default function handler(req: any, res: any) {
  res.status(200).json({ ok: true, router: "pages" });
}
