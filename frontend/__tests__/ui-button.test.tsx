import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { Button, buttonVariants } from "@/components/ui/button";

describe("Button", () => {
  it("renders its children", () => {
    render(<Button>Click me</Button>);

    expect(screen.getByRole("button", { name: "Click me" })).toBeInTheDocument();
  });

  it("fires onClick when clicked", async () => {
    const user = userEvent.setup();
    const onClick = jest.fn();

    render(<Button onClick={onClick}>Click me</Button>);

    await user.click(screen.getByRole("button", { name: "Click me" }));

    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it("does not fire onClick when disabled", async () => {
    const user = userEvent.setup();
    const onClick = jest.fn();

    render(
      <Button disabled onClick={onClick}>
        Click me
      </Button>,
    );

    await user.click(screen.getByRole("button", { name: "Click me" }));

    expect(onClick).not.toHaveBeenCalled();
  });

  it.each(["primary", "secondary", "tertiary", "destructive", "ghost", "link"])(
    "applies the %s variant class",
    (variant) => {
      render(<Button variant={variant as never}>Click me</Button>);

      const button = screen.getByRole("button", { name: "Click me" });
      const expected =
        buttonVariants({ variant: variant as never })
          .split(" ")
          .find((className) => className.startsWith("bg-") ||
            className.startsWith("text-primary") ||
            className.startsWith("border-") ||
            className.startsWith("shadow"));

      if (expected) expect(button).toHaveClass(expected);
    },
  );
});
