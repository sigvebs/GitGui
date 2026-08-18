fn main()
{
	println!("cargo:rerun-if-changed=assets/icon.ico");

	#[cfg(windows)]
	{
		let mut res = winresource::WindowsResource::new();
		res.set_icon("assets/icon.ico");
		res.set("FileDescription", "Git GUI");
		res.set("ProductName", "Git GUI");
		if let Err(err) = res.compile()
		{
			println!("cargo:warning=could not embed the icon resource: {err}");
		}
	}
}
