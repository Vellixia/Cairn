import { describe, it, expect } from "vitest"
import { render } from "@testing-library/react"
import { Badge } from "@/components/ui/badge"

describe("Badge", () => {
  it("renders a <span> not a <div>", () => {
    const { container } = render(<Badge>test</Badge>)
    const el = container.firstChild as HTMLElement
    expect(el.tagName).toBe("SPAN")
  })

  it("renders children", () => {
    const { getByText } = render(<Badge>hello</Badge>)
    expect(getByText("hello")).toBeInTheDocument()
  })

  it("applies variant classes", () => {
    const { container } = render(<Badge variant="destructive">warn</Badge>)
    const el = container.firstChild as HTMLElement
    expect(el.className).toMatch(/destructive/)
  })

  it("forwards extra className", () => {
    const { container } = render(<Badge className="my-custom">x</Badge>)
    const el = container.firstChild as HTMLElement
    expect(el.className).toContain("my-custom")
  })

  it("is valid HTML inside a <p> (no block element in inline context)", () => {
    // This would cause a hydration error if Badge rendered <div>.
    const { container } = render(
      <p>
        description <Badge>tag</Badge>
      </p>
    )
    expect(container.querySelector("span")).toBeInTheDocument()
    expect(container.querySelector("div")).toBeNull()
  })
})
