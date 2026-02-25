export default function handler(req, res) {
  res.status(200).json({ message: "Hello from Rex API!", method: req.method });
}
