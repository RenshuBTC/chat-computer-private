"""Finding a hand and its 21 joints in a camera picture.

The same steps as MediaPipe's hand tracker (its graphs and calculators are published; the numbers below come
from them), running Google's two openly licensed networks (Apache 2.0) on our own engine (nn.py):

1. Palm detection, while no hand is being followed. The whole picture, letterboxed to 192x192, goes through
   the palm network, which scores 2016 fixed anchor positions; the boxes and 7 palm keypoints above the
   threshold are merged (weighted non-maximum suppression), and the best palm becomes a square, rotated crop:
   rotated so the wrist-to-middle-finger line points up, then enlarged (x2.6) and moved towards the fingers.
2. Landmarks: the crop, resampled to 224x224, goes through the landmark network: 21 joints (x, y, and a
   relative depth z), how sure it is a hand is there, and which hand.
3. Tracking: the joints give the next picture's crop (x2.0 around them), so palm detection runs again only
   when a hand is lost, or (track) now and then to find a second hand. A palm detected where a hand is
   already followed is that hand again, not a new one (MediaPipe's multi-hand graph does the same).

Points are in the camera picture's pixels. Joint numbers: 0 wrist; thumb 1-4; index 5-8; middle 9-12;
ring 13-16; little 17-20 (the last of each is the fingertip).
"""
from __future__ import annotations

import math
from dataclasses import dataclass

import numpy as np
from PIL import Image

from . import nn, tflite

PALM_SIZE = 192
LANDMARK_SIZE = 224
MIN_PALM_SCORE = 0.5
MIN_PRESENCE = 0.5
# Indices of the joints MediaPipe uses to place the next crop (the wrist and the bases of the fingers).
CROP_JOINTS = [0, 1, 2, 3, 5, 6, 9, 10, 13, 14, 17, 18]


@dataclass
class Roi:
    """A square crop of the picture: centre and side in pixels, and its rotation (radians, clockwise)."""
    cx: float
    cy: float
    side: float
    rotation: float


@dataclass
class Palm:
    score: float
    box: tuple[float, float, float, float]      # centre x, centre y, width, height (pixels)
    keypoints: np.ndarray                       # (7, 2) pixels: 0 wrist centre, 2 middle finger base


@dataclass
class Hand:
    joints: np.ndarray       # (21, 3): x, y in pixels; z relative depth, in pixels like x (smaller = nearer)
    presence: float          # how sure the network is that this is a hand
    right: float             # how sure it's a right hand (in a camera's own, unmirrored picture: MediaPipe
                             # calls these "Right" for its test pictures of right hands)
    roi: Roi


@dataclass
class Track:
    """A hand being followed: its number (kept while it's followed), the next picture's crop, and the hand."""
    id: int
    roi: Roi
    hand: Hand


def ssd_anchors() -> np.ndarray:
    """The palm network's 2016 anchor centres (normalized x, y): two per cell of a 24x24 grid (stride 8), then
    six per cell of a 12x12 grid (three layers of stride 16, merged), as MediaPipe's SsdAnchorsCalculator makes
    them for this model (fixed anchor size, aspect ratio 1, offset 0.5)."""
    out = []
    for stride, per_cell in ((8, 2), (16, 6)):
        n = math.ceil(PALM_SIZE / stride)
        ys, xs = np.mgrid[0:n, 0:n]
        centres = np.stack([(xs + 0.5) / n, (ys + 0.5) / n], axis=-1).reshape(-1, 1, 2)
        out.append(np.repeat(centres, per_cell, axis=1).reshape(-1, 2))
    return np.concatenate(out).astype(np.float32)


def normalize_radians(a: float) -> float:
    return a - 2 * math.pi * math.floor((a + math.pi) / (2 * math.pi))


def crop(image: Image.Image, roi: Roi, size: int, edge: str = "replicate") -> np.ndarray:
    """The rotated square `roi` of the picture, resampled to size x size (bilinear), as floats from 0 to 1.

    The output pixel (u, v) shows the picture at roi's centre + (u/size - 1/2) side along the crop's x axis and
    (v/size - 1/2) side along its y axis, which is OpenCV's warp of the crop's corners (what MediaPipe uses):
    the same mapping, in Pillow's coordinates (pixel centres at +0.5). Outside the picture: the nearest edge
    pixel ("replicate", as for landmarks) or black ("zero", as for palm detection)."""
    cos, sin = math.cos(roi.rotation), math.sin(roi.rotation)
    s = roi.side / size
    a, b = s * cos, -s * sin
    d, e = s * sin, s * cos
    # OpenCV: src = A @ (u, v) + t, with t the crop's corner; Pillow: src' = A @ dst' + t + 1/2 - A @ (1/2, 1/2).
    tx = roi.cx - roi.side / 2 * (cos - sin)
    ty = roi.cy - roi.side / 2 * (sin + cos)
    img, ox, oy = image, 0.0, 0.0
    if edge == "replicate":
        img, ox, oy = _with_edges(image, roi)
    c = tx + 0.5 - (a + b) / 2 - ox
    f = ty + 0.5 - (d + e) / 2 - oy
    out = img.transform((size, size), Image.Transform.AFFINE, (a, b, c, d, e, f), resample=Image.Resampling.BILINEAR)
    return np.asarray(out, dtype=np.float32) * np.float32(1 / 255)


def _with_edges(image: Image.Image, roi: Roi) -> tuple[Image.Image, float, float]:
    """The picture, extended with copies of its edge pixels where the crop reaches past it (MediaPipe's
    BORDER_REPLICATE; Pillow can only fill with one colour). Returns it and where its corner was."""
    half = roi.side * 0.71 + 2                    # half the diagonal of the square, and a little
    w, h = image.size
    left, top = int(max(0, -(roi.cx - half))) + 1, int(max(0, -(roi.cy - half))) + 1
    right, bottom = int(max(0, roi.cx + half - w)) + 1, int(max(0, roi.cy + half - h)) + 1
    if left == top == right == bottom == 1:
        return image, 0.0, 0.0
    arr = np.pad(np.asarray(image), ((top, bottom), (left, right), (0, 0)), mode="edge")
    return Image.fromarray(arr), float(-left), float(-top)


class HandTracker:
    def __init__(self, palm_model: bytes, landmark_model: bytes, target_angle: float = math.pi / 2,
                 max_hands: int = 2) -> None:
        self.palm_net = nn.Net(tflite.load(palm_model))
        self.landmark_net = nn.Net(tflite.load(landmark_model))
        self.anchors = ssd_anchors()
        # MediaPipe's own graph says 90 degrees; its newer task graph passes 90 as radians. Tests use that to
        # compare with it exactly.
        self.target_angle = target_angle
        self.max_hands = max_hands
        self.roi: Roi | None = None
        self.tracks: list[Track] = []
        self._next_id = 1

    # ---------------------------------------------------------------- following hands
    def track(self, image: Image.Image, search: bool = False) -> list[tuple[int, Hand]]:
        """The hands in this picture, each with its number: the ones followed from the last picture and, with
        `search`, new ones that palm detection finds (up to max_hands). Searching costs about three times
        what following one hand does, so the caller decides how often."""
        kept: list[Track] = []
        for tr in self.tracks:
            hand = self.landmarks(image, tr.roi)
            if hand.presence < MIN_PRESENCE:
                continue
            if any(same_hand(hand, other.hand) for other in kept):
                continue                         # two crops that slid onto the same hand: one is enough
            tr.hand, tr.roi = hand, self.next_roi(hand.joints, image.size)
            kept.append(tr)
        self.tracks = kept
        if search and len(self.tracks) < self.max_hands:
            for palm in self.find_palms(image):
                if len(self.tracks) >= self.max_hands:
                    break
                if any(covers(tr.hand, palm) for tr in self.tracks):
                    continue                     # a hand already followed
                hand = self.landmarks(image, self.palm_roi(palm, image.size))
                if hand.presence < MIN_PRESENCE or any(same_hand(hand, tr.hand) for tr in self.tracks):
                    continue
                self.tracks.append(Track(self._next_id, self.next_roi(hand.joints, image.size), hand))
                self._next_id += 1
        return [(tr.id, tr.hand) for tr in self.tracks]

    # ---------------------------------------------------------------- following one hand
    def process(self, image: Image.Image) -> Hand | None:
        """The hand in this picture (the one followed from the last picture, or the best palm found)."""
        if self.roi is None:
            palms = self.find_palms(image)
            if not palms:
                return None
            self.roi = self.palm_roi(palms[0], image.size)
        hand = self.landmarks(image, self.roi)
        if hand.presence < MIN_PRESENCE:
            self.roi = None
            return None
        self.roi = self.next_roi(hand.joints, image.size)
        return hand

    def reset(self) -> None:
        self.roi = None
        self.tracks = []

    # ---------------------------------------------------------------- palms
    def find_palms(self, image: Image.Image) -> list[Palm]:
        w, h = image.size
        side = float(max(w, h))                      # the picture, letterboxed into a square (black bars)
        roi = Roi(w / 2, h / 2, side, 0.0)
        x = crop(image, roi, PALM_SIZE, edge="zero")[None]
        boxes, scores = self.palm_net(x)
        boxes, scores = boxes.reshape(-1, 18), scores.reshape(-1)
        score = 1.0 / (1.0 + np.exp(-np.clip(scores, -100.0, 100.0)))
        keep = np.flatnonzero(score >= MIN_PALM_SCORE)
        if keep.size == 0:
            return []
        raw, anchors = boxes[keep] / PALM_SIZE, self.anchors[keep]
        cx, cy = raw[:, 0] + anchors[:, 0], raw[:, 1] + anchors[:, 1]
        bw, bh = raw[:, 2], raw[:, 3]
        kps = raw[:, 4:18].reshape(-1, 7, 2) + anchors[:, None, :]
        dets = []
        for i in _weighted_nms(cx, cy, bw, bh, score[keep], kps):
            s, (x0, y0, x1, y1), kp = i
            # From the square (0..1) back to the picture.
            to_px = lambda u, v: (roi.cx + (u - 0.5) * side, roi.cy + (v - 0.5) * side)
            (px0, py0), (px1, py1) = to_px(x0, y0), to_px(x1, y1)
            kp_px = np.stack([roi.cx + (kp[:, 0] - 0.5) * side, roi.cy + (kp[:, 1] - 0.5) * side], axis=-1)
            dets.append(Palm(float(s), ((px0 + px1) / 2, (py0 + py1) / 2, px1 - px0, py1 - py0), kp_px))
        return dets

    def palm_roi(self, palm: Palm, size: tuple[int, int]) -> Roi:
        """The crop for the landmark network from a palm: turned so the wrist-to-middle-finger line points up,
        then made square, 2.6 times bigger, and moved half its height towards the fingers."""
        (x0, y0), (x1, y1) = palm.keypoints[0], palm.keypoints[2]
        rotation = normalize_radians(self.target_angle - math.atan2(-(y1 - y0), x1 - x0))
        cx, cy, w, h = palm.box
        return _transform(cx, cy, w, h, rotation, scale=2.6, shift_y=-0.5)

    # ---------------------------------------------------------------- landmarks
    def landmarks(self, image: Image.Image, roi: Roi) -> Hand:
        x = crop(image, roi, LANDMARK_SIZE)[None]
        lm, presence, handedness, _world = self.landmark_net(x)
        lm = lm.reshape(21, 3) / LANDMARK_SIZE
        cos, sin = math.cos(roi.rotation), math.sin(roi.rotation)
        u, v = lm[:, 0] - 0.5, lm[:, 1] - 0.5
        joints = np.empty((21, 3), dtype=np.float32)
        joints[:, 0] = roi.cx + (cos * u - sin * v) * roi.side
        joints[:, 1] = roi.cy + (sin * u + cos * v) * roi.side
        joints[:, 2] = lm[:, 2] / 0.4 * roi.side          # MediaPipe's depth scale (normalize_z 0.4)
        return Hand(joints, float(presence.reshape(-1)[0]), float(handedness.reshape(-1)[0]), roi)

    def next_roi(self, joints: np.ndarray, size: tuple[int, int]) -> Roi:
        """The next picture's crop from these joints: turned so the wrist-to-knuckles line points up, around
        the wrist and finger bases, twice their size and moved a tenth towards the fingers."""
        pts = joints[CROP_JOINTS, :2].astype(np.float64)
        wrist = pts[0]
        knuckles = ((pts[4] + pts[8]) / 2 + pts[6]) / 2           # bases of index and ring, then middle
        rotation = normalize_radians(math.pi / 2 - math.atan2(-(knuckles[1] - wrist[1]), knuckles[0] - wrist[0]))
        centre = (pts.max(axis=0) + pts.min(axis=0)) / 2
        back = -rotation
        c, s = math.cos(back), math.sin(back)
        d = pts - centre
        proj = np.stack([d[:, 0] * c - d[:, 1] * s, d[:, 0] * s + d[:, 1] * c], axis=-1)
        pc = (proj.max(axis=0) + proj.min(axis=0)) / 2
        w, h = proj.max(axis=0) - proj.min(axis=0)
        cr, sr = math.cos(rotation), math.sin(rotation)
        cx = pc[0] * cr - pc[1] * sr + centre[0]
        cy = pc[0] * sr + pc[1] * cr + centre[1]
        return _transform(cx, cy, w, h, rotation, scale=2.0, shift_y=-0.1)


def hand_size(joints: np.ndarray) -> float:
    """Wrist to middle knuckle, in the picture (pixels)."""
    return float(np.linalg.norm(joints[9, :2] - joints[0, :2])) or 1.0


def same_hand(a: Hand, b: Hand) -> bool:
    """Whether two sets of joints are the same hand: on average within a third of a hand size of each other."""
    d = np.linalg.norm(a.joints[:, :2] - b.joints[:, :2], axis=1).mean()
    return bool(d < 0.35 * max(hand_size(a.joints), hand_size(b.joints)))


def covers(hand: Hand, palm: Palm) -> bool:
    """Whether a detected palm is a hand already followed: its centre inside that hand's joints (their box,
    a fifth bigger)."""
    lo, hi = hand.joints[:, :2].min(axis=0), hand.joints[:, :2].max(axis=0)
    pad = (hi - lo) * 0.1
    cx, cy = palm.box[0], palm.box[1]
    return bool(lo[0] - pad[0] <= cx <= hi[0] + pad[0] and lo[1] - pad[1] <= cy <= hi[1] + pad[1])


def _transform(cx: float, cy: float, w: float, h: float, rotation: float, scale: float, shift_y: float) -> Roi:
    """MediaPipe's RectTransformationCalculator in pixels: move along the rotated y axis, make square (the
    longer side), scale."""
    cx += -h * shift_y * math.sin(rotation)
    cy += h * shift_y * math.cos(rotation)
    return Roi(float(cx), float(cy), float(max(w, h) * scale), float(rotation))


def _weighted_nms(cx, cy, w, h, scores, kps, iou: float = 0.3):
    """MediaPipe's weighted non-maximum suppression: the best box, averaged (by score) with every box that
    overlaps it by more than `iou`, then the same for the boxes left. Yields (score, box, keypoints) with boxes
    as (xmin, ymin, xmax, ymax), normalized."""
    x0, y0, x1, y1 = cx - w / 2, cy - h / 2, cx + w / 2, cy + h / 2
    order = list(np.argsort(-scores, kind="stable"))
    while order:
        top = order[0]
        ix0 = np.maximum(x0[order], x0[top])
        iy0 = np.maximum(y0[order], y0[top])
        ix1 = np.minimum(x1[order], x1[top])
        iy1 = np.minimum(y1[order], y1[top])
        inter = np.clip(ix1 - ix0, 0, None) * np.clip(iy1 - iy0, 0, None)
        union = w[order] * h[order] + w[top] * h[top] - inter
        sim = np.where(union > 0, inter / np.where(union > 0, union, 1), 0)
        group = [order[i] for i in range(len(order)) if sim[i] > iou]
        rest = [order[i] for i in range(len(order)) if sim[i] <= iou]
        if group:
            g = np.array(group)
            ws = scores[g]
            tot = ws.sum()
            box = (float((x0[g] * ws).sum() / tot), float((y0[g] * ws).sum() / tot),
                   float((x1[g] * ws).sum() / tot), float((y1[g] * ws).sum() / tot))
            kp = (kps[g] * ws[:, None, None]).sum(axis=0) / tot
        else:
            box, kp = (float(x0[top]), float(y0[top]), float(x1[top]), float(y1[top])), kps[top]
        yield float(scores[top]), box, kp
        if len(rest) == len(order):
            break
        order = rest
