from __future__ import annotations

import os
import json
from dataclasses import dataclass, field
from typing import Optional, Iterator


@dataclass
class Config:
    host: str = "localhost"
    port: int = 8080
    debug: bool = False
    tags: list[str] = field(default_factory=list)

    @classmethod
    def from_env(cls) -> Config:
        return cls(
            host=os.getenv("HOST", "localhost"),
            port=int(os.getenv("PORT", "8080")),
            debug=os.getenv("DEBUG", "").lower() in ("1", "true"),
        )

    def to_dict(self) -> dict:
        return {
            "host": self.host,
            "port": self.port,
            "debug": self.debug,
            "tags": self.tags,
        }


class RingBuffer:
    """Fixed-size circular buffer."""

    def __init__(self, capacity: int) -> None:
        self._buf: list[Optional[int]] = [None] * capacity
        self._head = 0
        self._tail = 0
        self._size = 0
        self.capacity = capacity

    def push(self, value: int) -> None:
        self._buf[self._tail] = value
        self._tail = (self._tail + 1) % self.capacity
        if self._size < self.capacity:
            self._size += 1
        else:
            self._head = (self._head + 1) % self.capacity

    def pop(self) -> Optional[int]:
        if self._size == 0:
            return None
        value = self._buf[self._head]
        self._head = (self._head + 1) % self.capacity
        self._size -= 1
        return value

    def __iter__(self) -> Iterator[int]:
        for i in range(self._size):
            yield self._buf[(self._head + i) % self.capacity]  # type: ignore

    def __len__(self) -> int:
        return self._size


def flatten(nested: list) -> list:
    """Recursively flatten a nested list."""
    result = []
    for item in nested:
        if isinstance(item, list):
            result.extend(flatten(item))
        else:
            result.append(item)
    return result


if __name__ == "__main__":
    cfg = Config.from_env()
    print(json.dumps(cfg.to_dict(), indent=2))

    buf = RingBuffer(4)
    for n in range(6):
        buf.push(n)
    print(list(buf))  # [2, 3, 4, 5]

    nested = [1, [2, [3, 4]], [5, 6]]
    print(flatten(nested))  # [1, 2, 3, 4, 5, 6]
