import { describe, it, expect } from "vitest"
import { render } from "@testing-library/react"
import { Badge } from "@/components/ui/badge"

// ItemDescription renders a <p>. Nesting a <Badge> (formerly <div>) inside it was
// the root cause of the v6.1.0 hydration error. This test pins the fix.
describe("Badge inside paragraph (hydration regression)", () => {
  it("does not nest a block element inside <p>", () => {
    const { container } = render(
      <p data-testid="desc">
        A memory <Badge>working</Badge>
      </p>
    )
    // A <div> inside <p> is invalid HTML and triggers React hydration errors.
    expect(container.querySelector("p > div")).toBeNull()
    expect(container.querySelector("p > span")).toBeInTheDocument()
  })
})
