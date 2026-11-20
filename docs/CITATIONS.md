# Citations

Algorithms and standards the code is derived from, as the master prompt
requires. Add an entry whenever an algorithm header comment cites a source.
Under ADR-0001 no PrusaSlicer (AGPL-3.0) code may be used; slicing will be
implemented from the papers listed here.

| Where | Method | Source |
| --- | --- | --- |
| `pxp-doc` recompute levels | Topological sort by wavefronts | A. B. Kahn, "Topological sorting of large networks", *Communications of the ACM* 5(11), 1962 |
| `pxp-doc` object IDs | UUIDv7 layout and monotonic counter (method 1) | IETF RFC 9562, *Universally Unique IDentifiers (UUIDs)*, 2024, §5.7 and §6.2 |
| `pxp-doc` ID randomness | SplitMix64 | G. L. Steele, D. Lea, C. H. Flood, "Fast splittable pseudorandom number generators", OOPSLA 2014; public-domain reference code by S. Vigna |
| `pxp-math` stable hash | FNV-1a, 64-bit | G. Fowler, L. C. Noll, K.-P. Vo, FNV hash (public domain) |
| `pxp-math` rotations | Rodrigues' rotation formula | O. Rodrigues, 1840; any linear-algebra text |
| `pxp-math` Csys | Gram–Schmidt orthonormalisation | standard |
| `pxp-units` conversions | Exact definitions of the inch, pound and pound-force | International yard and pound agreement (1959); NIST SP 811 |

## Freedom-to-operate reminder

Some constant-engagement and adaptive roughing methods are patented
(master prompt). Before a public release, run a freedom-to-operate check on
Stage 3 CAM methods and list the papers they were implemented from here.
