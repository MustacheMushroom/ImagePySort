import os
import shutil
from PIL import Image
import tkinter as tk
from tkinter import filedialog, messagebox

def get_date_taken(path):
    """Extract date taken from image metadata."""
    try:
        exif = Image.open(path)._getexif()
        if 36867 in exif:
            return exif[36867]
        return None
    except:
        return None

def sort_images(directory):
    """Sort images based on their taken date."""
    for root, _, files in os.walk(directory):
        for file in files:
            if file.lower().endswith(('.png', '.jpg', '.jpeg', '.gif', '.bmp')):
                file_path = os.path.join(root, file)
                date = get_date_taken(file_path)
                if date:
                    year, month = date[:4], date[5:7]
                    month_name = {
                        '01': 'Jan', '02': 'Feb', '03': 'Mar', '04': 'Apr', 
                        '05': 'May', '06': 'Jun', '07': 'Jul', '08': 'Aug',
                        '09': 'Sep', '10': 'Oct', '11': 'Nov', '12': 'Dec'
                    }.get(month, '')
                    
                    if not month_name:
                        continue
                    
                    year_folder = os.path.join(directory, year)
                    month_folder = os.path.join(year_folder, month_name)
                    
                    if not os.path.exists(year_folder):
                        os.mkdir(year_folder)
                    if not os.path.exists(month_folder):
                        os.mkdir(month_folder)
                    
                    shutil.move(file_path, os.path.join(month_folder, file))
    messagebox.showinfo("Info", "Images sorted successfully!")

def select_directory():
    """Open directory selection dialog and sort images in the selected directory."""
    directory = filedialog.askdirectory()
    if directory:
        sort_images(directory)

# Create the GUI window
app = tk.Tk()
app.title("Image Sorter")

frame = tk.Frame(app, padx=20, pady=20)
frame.pack(padx=10, pady=10)

label = tk.Label(frame, text="Select a directory to sort its images by date:")
label.pack(pady=10)

btn = tk.Button(frame, text="Select Directory", command=select_directory)
btn.pack(pady=10)

app.mainloop()
