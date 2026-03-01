export default function handler(req: any, res: any) {
  res.status(200).json({ message: "Hello from Rex API!", method: req.method });
}
